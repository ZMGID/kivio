#!/usr/bin/env python3
"""Wrap an already built macOS preview binary; this is not a release installer."""
from pathlib import Path
import plistlib
import shutil
import sys
import argparse

if sys.platform != "darwin":
    raise SystemExit("This development bundle is macOS-only.")
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--compact", action="store_true", help="Launch the debug preview at 680 × 520 logical pixels")
args = parser.parse_args()
native = Path(__file__).resolve().parents[1]
binary = native / "target/debug/kivio-native"
if not binary.is_file():
    raise SystemExit("Build first: cargo build --manifest-path native/Cargo.toml --locked")
name = "Kivio Native Compact" if args.compact else "Kivio Native"
bundle = native / "target" / f"{name}.app"
contents = bundle / "Contents"
(contents / "MacOS").mkdir(parents=True, exist_ok=True)
shutil.copy2(binary, contents / "MacOS/kivio-native")
executable = "kivio-native"
if args.compact:
    executable = "compact-launcher"
    launcher = contents / "MacOS" / executable
    launcher.write_text('#!/bin/sh\nexport KIVIO_PREVIEW_COMPACT=1\nexec "$(dirname "$0")/kivio-native"\n')
    launcher.chmod(0o755)
with (contents / "Info.plist").open("wb") as output:
    plistlib.dump({
        "CFBundleExecutable": executable,
        "CFBundleIdentifier": "com.zmair.kivio.gpui.preview" + (".compact" if args.compact else ""),
        "CFBundleName": name,
        "CFBundleDisplayName": name,
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": "0.1.0",
        "CFBundleVersion": "1",
        "NSHighResolutionCapable": True,
    }, output)
print(bundle)
