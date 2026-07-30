#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

echo "== capture (drives app; pauses for OAuth) =="
npx tsx capture/runner.ts "$@"

echo "== build scenes (trim + bt709) =="
nix-shell -p ffmpeg jq --run './recordings/build-scenes.sh'

echo "== manifest =="
npx tsx remotion/gen-manifest.ts

echo "== render =="
mkdir -p out
nix-shell -p chromium ffmpeg --run 'npx remotion render remotion/src/Root.tsx Reel out/strikehub-reel.mp4 --browser-executable=$(which chromium) --public-dir=remotion/public'

echo "== retag bt709 (ensure players read the HD matrix) =="
nix-shell -p ffmpeg --run '
  ffmpeg -y -loglevel error -i out/strikehub-reel.mp4 \
    -vf "scale=out_range=full:out_color_matrix=bt709,format=yuv420p" \
    -color_range pc -colorspace bt709 -color_primaries bt709 -color_trc bt709 \
    -c:v libx264 -crf 18 -preset medium -c:a copy -movflags +faststart out/strikehub-reel-bt709.mp4
  mv out/strikehub-reel-bt709.mp4 out/strikehub-reel.mp4
'

echo "done -> out/strikehub-reel.mp4"
