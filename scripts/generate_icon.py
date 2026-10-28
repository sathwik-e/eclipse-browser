#!/usr/bin/env python3
"""Build the macOS .icns from the supplied Eclipse artwork.

The 1024 px source is generated from assets/eclipse-icon.webp; this script only
uses macOS built-ins (sips + iconutil), so packaging still needs no Python
image libraries.
"""

import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "assets" / "eclipse-icon-1024.png"
ICONSET = ROOT / "scripts" / "AppIcon.iconset"
ICNS = ROOT / "Eclipse.app" / "Contents" / "Resources" / "AppIcon.icns"
PREVIEW = ROOT / "scripts" / "icon_512.png"

# (pixels, iconset filename). iconutil requires this exact set of names.
ICON_FILES = [
    (16, "icon_16x16.png"),
    (32, "icon_16x16@2x.png"),
    (32, "icon_32x32.png"),
    (64, "icon_32x32@2x.png"),
    (128, "icon_128x128.png"),
    (256, "icon_128x128@2x.png"),
    (256, "icon_256x256.png"),
    (512, "icon_256x256@2x.png"),
    (512, "icon_512x512.png"),
    (1024, "icon_512x512@2x.png"),
]


def resize(size: int, destination: Path) -> None:
    subprocess.run(
        [
            "sips",
            "-z",
            str(size),
            str(size),
            str(SOURCE),
            "--out",
            str(destination),
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def main() -> int:
    if not SOURCE.exists():
        print(f"missing icon source: {SOURCE}", file=sys.stderr)
        return 1

    shutil.rmtree(ICONSET, ignore_errors=True)
    ICONSET.mkdir(parents=True, exist_ok=True)

    for size, name in ICON_FILES:
        resize(size, ICONSET / name)

    shutil.copyfile(ICONSET / "icon_512x512.png", PREVIEW)

    ICNS.parent.mkdir(parents=True, exist_ok=True)
    res = subprocess.run(
        ["iconutil", "-c", "icns", str(ICONSET), "-o", str(ICNS)],
        capture_output=True,
    )
    if res.returncode != 0 or not ICNS.exists():
        import struct

        chunks = []
        ostype_map = [
            (b"icp4", "icon_16x16.png"),
            (b"ic11", "icon_16x16@2x.png"),
            (b"icp5", "icon_32x32.png"),
            (b"ic12", "icon_32x32@2x.png"),
            (b"ic07", "icon_128x128.png"),
            (b"ic13", "icon_128x128@2x.png"),
            (b"ic08", "icon_256x256.png"),
            (b"ic14", "icon_256x256@2x.png"),
            (b"ic09", "icon_512x512.png"),
            (b"ic10", "icon_512x512@2x.png"),
        ]
        for ostype, filename in ostype_map:
            data = (ICONSET / filename).read_bytes()
            chunks.append(ostype + struct.pack(">I", 8 + len(data)) + data)
        body = b"".join(chunks)
        ICNS.write_bytes(b"icns" + struct.pack(">I", 8 + len(body)) + body)
    print(f"Generated {PREVIEW.relative_to(ROOT)} and {ICNS.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
