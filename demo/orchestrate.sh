#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

echo "== capture =="
npx tsx capture/driver.ts "$@"

echo "== manifest =="
npx tsx remotion/gen-manifest.ts

echo "== render =="
mkdir -p out
nix-shell -p chromium --run 'npx remotion render remotion/src/Root.tsx Reel out/strikehub-reel.mp4 --browser-executable=$(which chromium) --public-dir=remotion/public'

echo "done -> out/strikehub-reel.mp4"
