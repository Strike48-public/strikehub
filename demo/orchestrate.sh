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
# High quality: --crf 16 (visually lossless for screen content) + max jpeg quality
# on the frame pipeline so text/UI stays crisp. The RGB capture path already
# gives correct color, so NO post re-encode/retag — that only added a second
# lossy generation (artifacts). The bt470bg matrix tag is cosmetic; players
# render it correctly. Add a lossless tag-only fixup if a target demands bt709.
nix-shell -p chromium ffmpeg --run 'npx remotion render remotion/src/Root.tsx Reel out/strikehub-reel.mp4 --browser-executable=$(which chromium) --public-dir=remotion/public --crf 16 --jpeg-quality 100'

echo "done -> out/strikehub-reel.mp4"
