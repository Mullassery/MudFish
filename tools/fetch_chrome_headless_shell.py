#!/usr/bin/env python3
"""Download a pinned ``chrome-headless-shell`` (Chrome for Testing) into the
Python package so a bundled wheel can ship a working ``render_js`` binary.

The binary and its sibling resources (``icudtl.dat``, ``libEGL.dylib``, ...)
are flattened into ``crates/python/python/mudfish/_chrome/`` so the runtime
lookup is a single fixed path. This directory is gitignored -- it is fetched
at bundle-build time, not committed.

Usage::

    python3 tools/fetch_chrome_headless_shell.py [--platform mac-arm64]

Writes a small manifest next to the binary recording the CfT version.
"""

from __future__ import annotations

import argparse
import json
import platform
import shutil
import sys
import urllib.request
import zipfile
from pathlib import Path

KNOWN_GOOD_URL = (
    "https://googlechromelabs.github.io/chrome-for-testing/"
    "last-known-good-versions-with-downloads.json"
)

REPO_ROOT = Path(__file__).resolve().parent.parent
DEST = REPO_ROOT / "crates" / "python" / "python" / "mudfish" / "_chrome"


def default_platform() -> str:
    system = platform.system()
    machine = platform.machine().lower()
    if system == "Darwin":
        return "mac-arm64" if machine in ("arm64", "aarch64") else "mac-x64"
    if system == "Linux":
        return "linux64"
    if system == "Windows":
        return "win64" if machine in ("amd64", "x86_64") else "win32"
    raise SystemExit(f"unsupported platform: {system}/{machine}")


def resolve_download(plat: str) -> tuple[str, str]:
    with urllib.request.urlopen(KNOWN_GOOD_URL, timeout=60) as resp:
        data = json.load(resp)
    stable = data["channels"]["Stable"]
    version = stable["version"]
    for entry in stable["downloads"].get("chrome-headless-shell", []):
        if entry["platform"] == plat:
            return version, entry["url"]
    raise SystemExit(f"no chrome-headless-shell download for platform {plat!r}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", default=default_platform())
    parser.add_argument("--dest", type=Path, default=DEST)
    args = parser.parse_args()

    version, url = resolve_download(args.platform)
    print(f"chrome-headless-shell {version} ({args.platform})")
    print(f"  <- {url}")

    if args.dest.exists():
        shutil.rmtree(args.dest)
    args.dest.mkdir(parents=True)

    with urllib.request.urlopen(url, timeout=300) as resp:
        payload = resp.read()

    import io

    with zipfile.ZipFile(io.BytesIO(payload)) as zf:
        top = {name.split("/")[0] for name in zf.namelist() if "/" in name}
        prefix = next(iter(top)) + "/"
        for member in zf.namelist():
            if not member.startswith(prefix) or member.endswith("/"):
                continue
            rel = member[len(prefix):]
            target = args.dest / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            with zf.open(member) as src, open(target, "wb") as out:
                shutil.copyfileobj(src, out)

    manifest = args.dest / "CHROME_HEADLESS_SHELL_VERSION"
    manifest.write_text(f"{version}\n{args.platform}\n")
    print(f"  -> {args.dest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
