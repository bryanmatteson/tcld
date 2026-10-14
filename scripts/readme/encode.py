#!/usr/bin/env python3
"""Encode actual UI captures listed in frames.json; publish stills and a GIF."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("capture_dir", type=Path)
args = parser.parse_args()
capture = args.capture_dir.expanduser().resolve()
root = Path(__file__).resolve().parents[2]
assets = root / 'docs/assets'
assets.mkdir(parents=True, exist_ok=True)
frames = json.loads((capture / "frames.json").read_text())
if not frames or any(not (capture / f["file"]).is_file() or f["duration"] <= 0 for f in frames):
    raise SystemExit("Missing capture or invalid frame duration")
concat = []
filters = ["scale=1280:-2:flags=lanczos"]
start = 0
for i, frame in enumerate(frames):
    image = capture / frame["file"]
    # ffconcat uses single-quoted paths with backslash escapes.
    path = str(image).replace("'", "'\\''")
    concat.extend([f"file '{path}'", f"duration {frame['duration']}"])
    end = start + frame["duration"]
    start = end
    shutil.copyfile(image, assets / image.name)
concat.append(concat[-2])
(capture / "frames.ffconcat").write_text("ffconcat version 1.0\n" + "\n".join(concat) + "\n")
filters.extend(["fps=5", "split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4"])
limiter = Path.home() / ".local/bin/agent-build"
prefix = [str(limiter), "--"] if limiter.is_file() else []
subprocess.run(prefix + ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-safe", "0", "-f", "concat", "-i", str(capture / "frames.ffconcat"), "-vf", ",".join(filters), "-t", str(start), "-loop", "0", str(capture / 'workbench.gif')], check=True)
shutil.copyfile(capture / 'workbench.gif', assets / 'workbench.gif')
print(f"Published {len(frames)} actual screenshots and {start:g}s of GIF walkthrough to {assets}")
