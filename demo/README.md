# StrikeHub demo video toolchain

Automated off-screen capture + Remotion compositing. See
`docs/superpowers/specs/2026-07-29-automated-demo-video-design.md`.

## Prereqs (Linux/Hyprland dev box)
- `npm install` in this dir
- Tools on PATH: hyprctl, grim, wf-recorder, ydotool (+ running ydotoold), jq, sway
- ffprobe via `nix-shell -p ffmpeg` (orchestrate.sh wraps this)
- Operator in the `ydotool` group
- StrikeHub built: `../target/debug/strikehub` (run `nix develop` build first)

## Run (deterministic harness)
- Full pipeline (drives app, pauses for OAuth, records, composites):
  `./orchestrate.sh`
- Re-resolve all click targets with the LLM (rebuild cache): `./orchestrate.sh --refresh`
- One scene: `npx tsx capture/runner.ts --scene easymode`
- Dry drive (no recording, for tuning): `npx tsx capture/runner.ts --no-record`
- Compose only (skip capture): `nix-shell -p ffmpeg jq --run './recordings/build-scenes.sh' && npm run manifest && ./orchestrate.sh`  (or run the render block)

Requirements: Hyprland session; operator in the `ydotool` group; `ANTHROPIC_API_KEY` in env; `../target/debug/strikehub` built. Click targets are resolved by Claude vision once and cached in `cache/actions.json` (committed) — subsequent runs replay deterministically with zero LLM `act()` calls.
