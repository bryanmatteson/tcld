#!/usr/bin/env python3
"""Build and bundle the actual embedded Tcld host for isolated macOS capture."""
import argparse
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--skip-build", action="store_true")
args = parser.parse_args()
if sys.platform != "darwin":
    raise SystemExit("This preparation script bundles the macOS app; other platforms can use their normal Tauri build.")
root = Path(__file__).resolve().parents[2]
output = Path.home() / "Documents/Developer/tcld/readme"
if not args.skip_build:
    limiter = Path.home() / ".local/bin/agent-build"
    prefix = [str(limiter), "--"] if limiter.exists() else []
    subprocess.run(prefix + ["npm", "ci", "--prefix", "apps/tcld-gui"], cwd=root, check=True)
    subprocess.run(prefix + ["npm", "run", "build", "--prefix", "apps/tcld-gui"], cwd=root, check=True)
    subprocess.run(["cargo", "build", "--release", "-p", "tcld-tauri", "--features", "tauri/custom-protocol"], cwd=root, check=True)
contents = output / "TCLD.app/Contents"
(contents / "MacOS").mkdir(parents=True, exist_ok=True)
(contents / "Resources").mkdir(parents=True, exist_ok=True)
shutil.copy2(root / "target/release/tcld-tauri", contents / "MacOS/TCLD")
shutil.copy2(root / "apps/tcld-tauri/icons/icon.icns", contents / "Resources/icon.icns")
with (contents / "Info.plist").open("wb") as file:
    plistlib.dump({"CFBundleExecutable": "TCLD", "CFBundleIdentifier": "dev.matteson.tcld.readme", "CFBundleName": "TCLD", "CFBundleDisplayName": "TCLD", "CFBundlePackageType": "APPL", "CFBundleIconFile": "icon.icns", "NSHighResolutionCapable": True}, file)
print(f"Prepared {output / 'TCLD.app'}")
print("Open the app and testing/fixtures/public/public_complex_fixture.tbc, then select the Default layout.")
