import json
from pathlib import Path

from mitmproxy import http

WIDEVINE_HOSTS = ["cpix.tbxdrm.com", "tbxdrm.com", "license"]
OUTPUT_PATH = Path("captured_license.json")

captured = {
    "license_request": None,
    "license_response": None,
    "license_url": None,
}


def is_license_url(url: str) -> bool:
    return any(h in url for h in WIDEVINE_HOSTS)


def to_hex(data: bytes) -> str:
    return data.hex()


def save():
    OUTPUT_PATH.write_text(json.dumps(captured, indent=2))
    print(f"\n[saved] {OUTPUT_PATH}")
    print(f"  request  : {len(captured['license_request'] or '') // 2} bytes")
    print(f"  response : {len(captured['license_response'] or '') // 2} bytes")
    print(f"  url      : {captured['license_url']}")
    print("\n[next] Share captured_license.json to extract Widevine keys")


def request(flow: http.HTTPFlow) -> None:
    if not is_license_url(flow.request.pretty_url):
        return

    url = flow.request.pretty_url
    body = flow.request.content

    print(f"\n[+] License REQUEST intercepted")
    print(f"    url  : {url}")
    print(f"    size : {len(body)} bytes")

    captured["license_url"] = url
    captured["license_request"] = to_hex(body)


def response(flow: http.HTTPFlow) -> None:
    if not is_license_url(flow.request.pretty_url):
        return

    body = flow.response.content
    status = flow.response.status_code

    print(f"\n[+] License RESPONSE intercepted")
    print(f"    status : {status}")
    print(f"    size   : {len(body)} bytes")

    captured["license_response"] = to_hex(body)
    save()
