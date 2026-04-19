import base64
import json
import sys
import xml.etree.ElementTree as ET

import requests

TBXAPIS_BASE = "https://unity.tbxapis.com"
CONTENT_ID = "692dc3d7ddd30f329e90a4cc"
WIDEVINE_SYSTEM_ID = "edef8ba9-79d6-4ace-a3c8-27dcd51d21ed"

BROWSER_HEADERS = {
    "accept": "application/json",
    "accept-language": "es,en-US;q=0.9,en;q=0.8",
    "content-type": "application/json",
    "origin": "https://winplay.co",
    "referer": "https://winplay.co/",
    "sec-ch-ua": '"Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147"',
    "sec-ch-ua-mobile": "?0",
    "sec-ch-ua-platform": '"macOS"',
    "sec-fetch-dest": "empty",
    "sec-fetch-mode": "cors",
    "sec-fetch-site": "cross-site",
    "user-agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36",
}

OK = "\033[92m[ok]\033[0m"
FAIL = "\033[91m[fail]\033[0m"
INFO = "\033[94m[info]\033[0m"


def step(n, total, msg):
    print(f"\n\033[1m[{n}/{total}]\033[0m {msg}")


def test_fetch_stream_urls(jwt_token):
    step(1, 5, "fetching stream URLs from tbxapis...")
    headers = {**BROWSER_HEADERS, "authorization": f"JWT {jwt_token}"}
    url = f"{TBXAPIS_BASE}/v0/contents/{CONTENT_ID}/url"
    r = requests.get(url, headers=headers, timeout=15)
    print(f"  status: {r.status_code}")
    r.raise_for_status()
    data = r.json()
    print(f"  {OK} content title: {data.get('content', {}).get('title')}")
    return data


def test_extract_entitlements(stream_data):
    step(2, 5, "extracting entitlements...")
    entitlements = stream_data.get("content", {}).get(
        "entitlements"
    ) or stream_data.get("entitlements", [])
    dash = None
    hls = None
    for e in entitlements:
        if e.get("contentType") == "application/dash+xml":
            dash = e
        if e.get("contentType") == "application/x-mpegURL":
            hls = e

    if dash:
        print(f"  {OK} DASH URL: {dash['url'][:90]}...")
        print(
            f"  {OK} Widevine license URL: {dash['drm']['widevine']['licenseAcquisitionUrl'][:90]}..."
        )
    else:
        print(f"  {FAIL} no DASH entitlement found")

    if hls:
        print(f"  {INFO} HLS URL (FairPlay): {hls['url'][:90]}...")

    return dash


def test_fetch_mpd(dash_entitlement):
    step(3, 5, "downloading MPD manifest...")
    mpd_url = dash_entitlement["url"]
    headers = {
        "accept": "*/*",
        "origin": "https://winplay.co",
        "referer": "https://winplay.co/",
        "user-agent": BROWSER_HEADERS["user-agent"],
    }
    r = requests.get(mpd_url, headers=headers, timeout=15)
    print(f"  status: {r.status_code}")
    r.raise_for_status()
    print(f"  {OK} MPD size: {len(r.text)} bytes")
    return r.text


def test_extract_pssh(mpd_xml):
    step(4, 5, "extracting Widevine PSSH from MPD...")
    root = ET.fromstring(mpd_xml)
    ns = {
        "mpd": "urn:mpeg:dash:schema:mpd:2011",
        "cenc": "urn:mpeg:cenc:2013",
    }

    found = []
    for pssh_elem in root.findall(".//cenc:pssh", ns):
        raw = base64.b64decode(pssh_elem.text.strip())
        system_id_bytes = raw[12:28]
        system_id = "-".join(
            [
                system_id_bytes[0:4].hex(),
                system_id_bytes[4:6].hex(),
                system_id_bytes[6:8].hex(),
                system_id_bytes[8:10].hex(),
                system_id_bytes[10:16].hex(),
            ]
        )
        found.append((system_id, pssh_elem.text.strip()))
        label = "Widevine" if system_id.lower() == WIDEVINE_SYSTEM_ID else "other"
        print(f"  {OK} PSSH [{label}] system_id={system_id}")
        print(f"       data={pssh_elem.text.strip()[:60]}...")

    if not found:
        print(f"  {FAIL} no PSSH found in MPD")
        return None

    widevine_pssh = next((p for s, p in found if s.lower() == WIDEVINE_SYSTEM_ID), None)
    return widevine_pssh


def test_probe_license_server(dash_entitlement):
    step(5, 5, "probing Widevine license server (empty POST)...")
    license_url = dash_entitlement["drm"]["widevine"]["licenseAcquisitionUrl"]
    r = requests.post(
        license_url,
        data=b"",
        headers={
            "content-type": "application/octet-stream",
            "origin": "https://winplay.co",
            "referer": "https://winplay.co/",
            "user-agent": BROWSER_HEADERS["user-agent"],
        },
        timeout=15,
    )
    print(f"  status: {r.status_code}")
    if r.status_code in (400, 403):
        print(f"  {OK} license server reachable (rejected empty challenge as expected)")
    elif r.status_code == 200:
        print(f"  {OK} license server returned 200 (unexpected for empty challenge)")
    else:
        print(f"  {INFO} license server returned {r.status_code}: {r.text[:200]}")


def run(jwt_token):
    print("\n\033[1m=== WinPlay DASH stream flow test ===\033[0m")

    stream_data = test_fetch_stream_urls(jwt_token)
    dash = test_extract_entitlements(stream_data)
    if not dash:
        sys.exit(1)

    mpd_xml = test_fetch_mpd(dash)
    pssh = test_extract_pssh(mpd_xml)
    test_probe_license_server(dash)

    print("\n\033[1m=== summary ===\033[0m")
    print(f"  MPD URL:     {dash['url'][:80]}...")
    print(f"  License URL: {dash['drm']['widevine']['licenseAcquisitionUrl'][:80]}...")
    print(f"  PSSH:        {(pssh or 'NOT FOUND')[:60]}...")
    print(
        f"\n  {OK} ready for key extraction — run get_keys.py with a .wvd device file"
    )

    with open("flow_test_result.json", "w") as f:
        json.dump(
            {
                "mpd_url": dash["url"],
                "license_url": dash["drm"]["widevine"]["licenseAcquisitionUrl"],
                "pssh": pssh,
            },
            f,
            indent=2,
        )
    print(f"  {INFO} results saved to flow_test_result.json")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(f"usage: python3 {sys.argv[0]} <jwt_token>")
        sys.exit(1)
    run(sys.argv[1])
