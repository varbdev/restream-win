import json
import signal
import sys
import time
from pathlib import Path

import frida

TARGET_KID = "19ca642b3eba4f4d81637667a04fdd9e"
OUTPUT_KEYS_PATH = Path("keys.json")
OUTPUT_CAPTURE_PATH = Path("captured_license.json")

EME_HOOK_JS = r"""
'use strict';

function toHex(input) {
    var u8 = input instanceof Uint8Array ? input : new Uint8Array(input instanceof ArrayBuffer ? input : input.buffer);
    return Array.from(u8).map(function(b) { return ('0' + b.toString(16)).slice(-2); }).join('');
}

var hooked = false;

function installHooks() {
    if (hooked || typeof MediaKeySession === 'undefined') return false;
    hooked = true;

    var origGenerateRequest = MediaKeySession.prototype.generateRequest;
    MediaKeySession.prototype.generateRequest = function(initDataType, initData) {
        try { send({ type: 'pssh', initDataType: initDataType, data: toHex(initData) }); } catch(e) {}
        return origGenerateRequest.apply(this, arguments);
    };

    var origUpdate = MediaKeySession.prototype.update;
    MediaKeySession.prototype.update = function(response) {
        try { send({ type: 'license_response', data: toHex(response) }); } catch(e) {}
        return origUpdate.apply(this, arguments);
    };

    var origFetch = window.fetch;
    window.fetch = function(input, init) {
        var url = typeof input === 'string' ? input : (input && input.url) || '';
        if (init && init.body && (url.indexOf('tbxdrm') !== -1 || url.indexOf('license') !== -1 || url.indexOf('cpix') !== -1)) {
            try {
                var b = init.body instanceof ArrayBuffer ? init.body : null;
                if (b) send({ type: 'license_request', url: url, data: toHex(b) });
            } catch(e) {}
        }
        var p = origFetch.apply(this, arguments);
        if (url.indexOf('tbxdrm') !== -1 || url.indexOf('license') !== -1 || url.indexOf('cpix') !== -1) {
            p = p.then(function(resp) {
                resp.clone().arrayBuffer().then(function(buf) {
                    send({ type: 'license_response_http', url: url, data: toHex(buf) });
                });
                return resp;
            });
        }
        return p;
    };

    send({ type: 'ready' });
    return true;
}

if (!installHooks()) {
    var attempts = 0;
    var poller = setInterval(function() {
        if (installHooks() || attempts++ > 30) clearInterval(poller);
    }, 500);
}
"""

MEMORY_SCAN_JS = r"""
'use strict';

function toHex(ab) {
    return Array.from(new Uint8Array(ab)).map(function(b) { return ('0' + b.toString(16)).slice(-2); }).join('');
}

function hexToPattern(hex) {
    var parts = [];
    for (var i = 0; i < hex.length; i += 2) parts.push(hex.substr(i, 2));
    return parts.join(' ');
}

function isBlank(hex) {
    return hex === '00000000000000000000000000000000' || hex === 'ffffffffffffffffffffffffffffffff';
}

function scanForKid(kidHex) {
    var pattern = hexToPattern(kidHex);
    var total = 0;
    var ranges = Process.enumerateRanges({ protection: 'r--', coalesce: true });

    ranges.forEach(function(range) {
        if (range.size < 32 || range.size > 256 * 1024 * 1024) return;
        try {
            Memory.scan(range.base, range.size, pattern, {
                onMatch: function(address) {
                    var candidates = [];
                    [-64, -48, -32, -16, 16, 32, 48, 64].forEach(function(off) {
                        try {
                            var buf = Memory.readByteArray(address.add(off), 16);
                            var hex = toHex(buf);
                            if (!isBlank(hex)) {
                                candidates.push({ offset: off, key: hex });
                            }
                        } catch(e) {}
                    });
                    if (candidates.length > 0) {
                        total++;
                        send({ type: 'key_candidate', kid: kidHex, address: address.toString(), candidates: candidates });
                    }
                },
                onError: function() {}
            });
        } catch(e) {}
    });

    send({ type: 'scan_complete', kid: kidHex, total: total });
}

rpc.exports.scan = function(kid) { scanForKid(kid); };
"""


def discover_chrome_processes():
    device = frida.get_local_device()
    return [
        p
        for p in device.enumerate_processes()
        if "Google Chrome" in p.name or "Chromium" in p.name
    ]


def find_cdm_process(procs):
    for proc in procs:
        found = []
        try:
            sess = frida.attach(proc.pid)
            sc = sess.create_script(
                "send(Process.enumerateModules().map(function(m){return m.name;}).join(','));"
            )
            sc.on("message", lambda m, d: found.append(m.get("payload", "")))
            sc.load()
            time.sleep(0.4)
            sc.unload()
            sess.detach()
            if found and "libwidevinecdm" in found[0]:
                return proc
        except Exception:
            continue
    return None


def attach_eme_hooks(procs, on_message):
    sessions = []
    for proc in procs:
        try:
            sess = frida.attach(proc.pid)
            sc = sess.create_script(EME_HOOK_JS)
            sc.on("message", on_message)
            sc.load()
            print(f"[+] EME hooks injected → pid={proc.pid} ({proc.name})")
            sessions.append((sess, sc))
        except Exception as ex:
            print(f"[-] Skip pid={proc.pid}: {ex}")
    return sessions


def attach_memory_scanner(proc, on_message):
    try:
        sess = frida.attach(proc.pid)
        sc = sess.create_script(MEMORY_SCAN_JS)
        sc.on("message", on_message)
        sc.load()
        print(f"[+] Memory scanner injected → pid={proc.pid} ({proc.name})")
        return sess, sc
    except Exception as ex:
        print(f"[-] Scanner attach failed: {ex}")
        return None, None


def save_keys(kid, key):
    data = {"keys": [{"kid": kid, "key": key}], "mpd_url": "", "license_url": ""}
    OUTPUT_KEYS_PATH.write_text(json.dumps(data, indent=2))
    print(f"\n[saved] {OUTPUT_KEYS_PATH}")


def save_capture(captured):
    OUTPUT_CAPTURE_PATH.write_text(json.dumps(captured, indent=2))
    print(f"[saved] {OUTPUT_CAPTURE_PATH}")


def build_interrupt_handler(scanner_script, key_candidates, captured):
    def handle(sig, frame):
        print("\n[*] Scanning memory for KID...")
        if scanner_script:
            try:
                scanner_script.exports.scan(TARGET_KID)
                time.sleep(5)
            except Exception as ex:
                print(f"[-] Scan error: {ex}")

        if key_candidates:
            print(f"\n[+] {len(key_candidates)} key candidate(s) found:")
            for c in key_candidates:
                print(f"    {c['kid']}:{c['key']}  (offset {c['offset']:+d})")
            best = key_candidates[0]
            save_keys(best["kid"], best["key"])
            print(
                "\n[*] Now restart the restream server — it will load keys.json automatically"
            )
        else:
            print("[!] No keys found in memory")
            if any(v for v in captured.values() if v):
                save_capture(captured)
                print(f"[*] License bytes saved to {OUTPUT_CAPTURE_PATH}")
                print(
                    "[*] Share captured_license.json with a Widevine CDM service to get keys"
                )
        sys.exit(0)

    return handle


def build_eme_handler(captured):
    def handle(message, data):
        if message["type"] != "send":
            return
        payload = message["payload"]
        t = payload.get("type")
        if t == "ready":
            print("[hook] EME hooks active in renderer")
        elif t == "pssh":
            size = len(payload.get("data", "")) // 2
            print(f"[+] PSSH captured ({payload.get('initDataType')}) {size}B")
            captured["pssh"] = payload.get("data")
        elif t == "license_request":
            url = payload.get("url", "")
            print(f"[+] License request → {url[:90]}")
            captured["license_request"] = {"url": url, "data": payload.get("data")}
        elif t in ("license_response", "license_response_http"):
            size = len(payload.get("data", "")) // 2
            print(f"[+] License response {size}B")
            captured["license_response"] = payload.get("data")
            save_capture(captured)

    return handle


def build_scan_handler(key_candidates):
    def handle(message, data):
        if message["type"] != "send":
            return
        payload = message["payload"]
        t = payload.get("type")
        if t == "key_candidate":
            print(f"\n[!!!] KID match @ {payload.get('address')}")
            for c in payload.get("candidates", []):
                print(f"      offset={c['offset']:+4d} → {c['key']}")
            for c in payload.get("candidates", []):
                key_candidates.append(
                    {"kid": TARGET_KID, "key": c["key"], "offset": c["offset"]}
                )
        elif t == "scan_complete":
            n = payload.get("total", 0)
            print(f"[*] Memory scan complete — KID occurrences: {n}")

    return handle


def run():
    procs = discover_chrome_processes()
    if not procs:
        print(
            "[error] Chrome not running. Open Chrome and navigate to winplay.co first."
        )
        sys.exit(1)

    print(f"[*] Chrome processes: {len(procs)}")
    for p in procs:
        print(f"    pid={p.pid}  {p.name}")

    print("\n[*] Searching for process with Widevine CDM loaded...")
    cdm_proc = find_cdm_process(procs)

    if cdm_proc:
        print(f"[+] CDM process: pid={cdm_proc.pid}")
    else:
        print("[!] CDM not loaded. Start playing Win Sports in Chrome, then re-run.")
        print("[*] Attaching EME hooks to all Chrome processes anyway...\n")
        cdm_proc = procs[0]

    captured = {"pssh": None, "license_request": None, "license_response": None}
    key_candidates = []

    eme_handler = build_eme_handler(captured)
    scan_handler = build_scan_handler(key_candidates)

    attach_eme_hooks(procs, eme_handler)
    scanner_sess, scanner_script = attach_memory_scanner(cdm_proc, scan_handler)

    interrupt_handler = build_interrupt_handler(
        scanner_script, key_candidates, captured
    )
    signal.signal(signal.SIGINT, interrupt_handler)

    print(f"\n[*] Waiting for DRM events...")
    print(f"    1. Open Chrome → winplay.co → play Win Sports")
    print(f"    2. Wait for stream to start loading")
    print(f"    3. Press Ctrl+C to trigger memory scan")
    print(f"\n    Target KID : {TARGET_KID}")
    print(f"    License URL: cpix.tbxdrm.com/v1/license/winsports/widevine\n")

    while True:
        time.sleep(1)


if __name__ == "__main__":
    run()
