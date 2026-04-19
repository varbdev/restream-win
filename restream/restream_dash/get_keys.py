import base64
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

import requests
from pywidevine.cdm import Cdm
from pywidevine.device import Device
from pywidevine.pssh import PSSH

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


def load_jwt(jwt_token: str) -> dict:
    return {**BROWSER_HEADERS, "authorization": f"JWT {jwt_token}"}


def fetch_stream_urls(jwt_token: str) -> dict:
    headers = load_jwt(jwt_token)
    url = f"{TBXAPIS_BASE}/v0/contents/{CONTENT_ID}/url"
    response = requests.get(url, headers=headers, timeout=15)
    response.raise_for_status()
    return response.json()


def extract_dash_entitlement(stream_data: dict) -> dict:
    entitlements = stream_data.get("content", {}).get(
        "entitlements"
    ) or stream_data.get("entitlements", [])
    for entry in entitlements:
        if entry.get("contentType") == "application/dash+xml":
            return entry
    raise RuntimeError("no DASH entitlement found in stream data")


def fetch_mpd(mpd_url: str) -> str:
    headers = {
        "accept": "*/*",
        "origin": "https://winplay.co",
        "referer": "https://winplay.co/",
        "user-agent": BROWSER_HEADERS["user-agent"],
    }
    response = requests.get(mpd_url, headers=headers, timeout=15)
    response.raise_for_status()
    return response.text


def extract_widevine_pssh(mpd_xml: str) -> PSSH:
    root = ET.fromstring(mpd_xml)
    ns = {
        "mpd": "urn:mpeg:dash:schema:mpd:2011",
        "cenc": "urn:mpeg:cenc:2013",
    }

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
        if system_id.lower() == WIDEVINE_SYSTEM_ID:
            return PSSH(pssh_elem.text.strip())

    raise RuntimeError("no Widevine PSSH found in MPD")


def request_license(challenge: bytes, license_url: str) -> bytes:
    response = requests.post(
        license_url,
        data=challenge,
        headers={
            "content-type": "application/octet-stream",
            "origin": "https://winplay.co",
            "referer": "https://winplay.co/",
            "user-agent": BROWSER_HEADERS["user-agent"],
        },
        timeout=15,
    )
    response.raise_for_status()
    return response.content


def get_content_keys(
    pssh: PSSH, license_url: str, device_path: Path
) -> list[tuple[str, str]]:
    device = Device.load(device_path)
    cdm = Cdm.from_device(device)
    session_id = cdm.open()

    challenge = cdm.get_license_challenge(session_id, pssh)
    license_response = request_license(challenge, license_url)

    cdm.parse_license(session_id, license_response)
    keys = [
        (key.kid.hex, key.key.hex())
        for key in cdm.get_keys(session_id)
        if key.type == "CONTENT"
    ]
    cdm.close(session_id)
    return keys


def print_keys(keys: list[tuple[str, str]]) -> None:
    print("\n[keys]")
    for kid, key in keys:
        print(f"  {kid}:{key}")

    print("\n[mp4decrypt args]")
    args = []
    for kid, key in keys:
        args.append(f"--key {kid}:{key}")
    print("  " + " ".join(args))


def run(jwt_token: str, device_path: Path) -> None:
    print("[1/5] fetching stream URLs from tbxapis...")
    stream_data = fetch_stream_urls(jwt_token)
    entitlement = extract_dash_entitlement(stream_data)

    mpd_url = entitlement["url"]
    license_url = entitlement["drm"]["widevine"]["licenseAcquisitionUrl"]

    print(f"[2/5] MPD URL: {mpd_url[:80]}...")
    print(f"      license: {license_url[:80]}...")

    print("[3/5] fetching MPD and extracting PSSH...")
    mpd_xml = fetch_mpd(mpd_url)
    pssh = extract_widevine_pssh(mpd_xml)
    print(f"      PSSH: {str(pssh)[:60]}...")

    print("[4/5] requesting Widevine license...")
    keys = get_content_keys(pssh, license_url, device_path)

    print(f"[5/5] got {len(keys)} content key(s)")
    print_keys(keys)

    results = {
        "mpd_url": mpd_url,
        "license_url": license_url,
        "keys": [{"kid": kid, "key": key} for kid, key in keys],
    }
    output_path = Path("keys.json")
    import json

    output_path.write_text(json.dumps(results, indent=2))
    print(f"\n[saved] {output_path}")


if __name__ == "__main__":
    if len(sys.argv) < 3:
        print(f"usage: python3 {sys.argv[0]} <jwt_token> <device.wvd>")
        sys.exit(1)

    jwt_token = sys.argv[1]
    device_path = Path(sys.argv[2])

    if not device_path.exists():
        print(f"error: device file not found: {device_path}")
        sys.exit(1)

    run(jwt_token, device_path)
