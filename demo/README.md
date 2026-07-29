# StrikeHub demo video toolchain

Automated off-screen capture + Remotion compositing. See
`docs/superpowers/specs/2026-07-29-automated-demo-video-design.md`.

## Prereqs (Linux/Hyprland dev box)
- `npm install` in this dir
- Tools on PATH: hyprctl, grim, wf-recorder, ydotool (+ running ydotoold), jq, sway
- ffprobe via `nix-shell -p ffmpeg` (orchestrate.sh wraps this)
- Operator in the `ydotool` group
- StrikeHub built: `../target/debug/strikehub` (run `nix develop` build first)

## Run
- Full pipeline: `./orchestrate.sh`
- One scene: `npm run capture -- --scene launch`
- Dry run (no input sent): `npm run capture -- --dry-run`
- Probe targeting: `npm run capture -- --probe`
- Compose only: `npm run manifest && npm run render`
