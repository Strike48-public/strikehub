# Automated Demo Video Capture — Design

**Date:** 2026-07-29
**Status:** Approved design, ready for implementation planning
**Author:** Josh Adams (with Claude)

## Goal

Produce a ~60–90s feature reel showing off the whole StrikeHub app, built from
**real, automated screen captures** of the running application, composited into a
polished captioned video with **Remotion**. Capture must run **off the visible
screen** so the operator can keep using the machine without disturbing (or
appearing in) the recording.

Initial target platform: **Linux** (the dev machine). The architecture must let
macOS/Windows drop in later without structural change.

## Non-goals

- No voiceover, no music. **On-screen captions only** (plays clean muted).
- Not rebuilding the UI in React — footage is the real binary.
- Not shipping this in the app; `demo/` is standalone tooling.
- Not solving macOS/Windows capture now (SSH-only Windows boxes have no
  interactive desktop; no Mac available this session). Design leaves seams for them.

## Key context / constraints (validated live 2026-07-29)

The Linux app is Dioxus-desktop → WRY → **WebKitGTK**, on a **NixOS + Hyprland**
(wlroots, Wayland) machine with a 4K eDP-1 at scale 2.0. Capturing it headless
required solving several real issues; all of the following are **verified working**:

1. **Off-screen isolation:** `hyprctl output create headless` makes a virtual
   output (`HEADLESS-N`, 1920×1080) parked off-screen at `@5000,0`. Real GPU, so
   rendering is correct. Outputs **leak** — must be removed each run, and removing
   one shifts `eDP-1`, so reset `eDP-1,...,0x0,2` after.
2. **X11 mode is mandatory for correct rendering:** `GDK_BACKEND=x11` +
   `WEBKIT_DISABLE_DMABUF_RENDERER=1`. Under native Wayland, WebKit computes
   negative dimensions and the layout collapses to tiny/unstyled.
3. **X socket fix:** Hyprland's XWayland socket is `/tmp/.X11-unix/X0_` (trailing
   underscore); Xlib `DISPLAY=:0` expects `X0`. Symlink
   `ln -sf /tmp/.X11-unix/X0_ /tmp/.X11-unix/X0`. Without it,
   `global_hotkey::GlobalHotKeyManager::new` segfaults at `XDefaultRootWindow(NULL)`
   (dioxus-desktop registers global hotkeys and that crate is X11-only on Linux).
4. **Runtime deps:** launch inside `nix develop` — the binary links `libxdo.so.4`
   (+ webkitgtk/gtk) only present on the devshell `LD_LIBRARY_PATH`.
5. **Window placement:** window class is **`Strikehub`** (capital S), `xwayland=true`.
   Place with `hyprctl --batch "dispatch focuswindow address:$ADDR ; dispatch
   movewindow mon:HEADLESS-N"` then `dispatch fullscreen 1`. (Combined
   `movewindow "mon:X,address:Y"` does not parse — focus first, then move.)
6. **Capture:** `grim -o HEADLESS-N` (stills) / `wf-recorder -o HEADLESS-N` (video).
   The Hyprland top bar can overlay the output; crop to the window geometry.
7. **Input injection:** `ydotool` via the running `ydotoold`
   (`YDOTOOL_SOCKET=/run/ydotoold/socket`), invoked through `sg ydotool -c '…'`
   (operator is in the `ydotool` group; `sg` avoids a re-login). **Coordinate
   calibration:** ydotool absolute coords are in *scaled* space —
   `ydotool_coord = logical_coord ÷ monitor_scale` (scale 2.0 here). Verified exact.

Backend / auth: app points at `plg.strike48.test` (baked default) with
`MATRIX_TLS_INSECURE=1` for the dev cert.

## Architecture

Standalone `demo/` directory — a self-contained Node project that never touches
the Rust workspace. Four sequential, independently-runnable stages:

```
demo/
  orchestrate.sh          # run the whole pipeline end-to-end
  env/
    session.sh            # resolve HYPRLAND_INSTANCE_SIGNATURE, XDG_RUNTIME_DIR,
                          #   ydotool socket, devshell env; export for children
    stage.sh              # create+park headless output, X0 symlink, cleanup trap
  capture/
    calibrate.ts          # read monitor scale via hyprctl -> logical<->device transform
    actions.ts            # click/move/type/key/wait — ydotool wrapper (via sg), scaled
    window.ts             # find Strikehub window (hyprctl clients), move to headless,
                          #   fullscreen, return its geometry
    driver.ts             # launch app (nix develop, X11 env), run flow, record per scene
    flow.ts               # THE SCRIPT: ordered scenes {name, steps[], caption, kenBurns?}
  recordings/             # raw per-scene clips (gitignored)
  remotion/               # Remotion React project: clips + captions + bookends -> mp4
    src/Root.tsx, Reel.tsx, Scene.tsx, Caption.tsx, Intro.tsx, Outro.tsx
    manifest.json         # generated: [{clip, caption, durationInFrames, kenBurns}]
  out/                    # final rendered video (gitignored)
  README.md
```

**Data flow:** `flow.ts` (scene list) is the single source of truth. `driver.ts`
launches StrikeHub onto the headless output, and for each scene: starts
`wf-recorder` cropped to the window, replays that scene's `steps` via `actions.ts`,
stops the recorder → one clip per scene in `recordings/`. A generator writes
`manifest.json` (clip path + caption + measured duration via ffprobe). Remotion
reads the manifest and renders the final captioned reel to `out/`.

**Why staged:** capture and compositing are fully decoupled. Re-run only Remotion
to tweak captions/timing without re-recording; re-capture one scene with
`--scene <name>` without re-rendering everything.

## Components

### env/session.sh, env/stage.sh
- `session.sh`: resolves the live Hyprland instance signature (the socket that
  answers `hyprctl version`), `XDG_RUNTIME_DIR=/run/user/$(id -u)`, the ydotool
  socket, and captures the `nix develop` env once to a file for reuse.
- `stage.sh`: idempotent setup — ensure `X0` symlink; create + park a headless
  output; register a cleanup **trap** that kills the app, removes *all* HEADLESS-*
  outputs, and resets `eDP-1,...,0x0,2`. Every run cleans up after itself.

### capture/calibrate.ts
Pure function. Reads `hyprctl monitors -j` for the headless output's `scale`, size,
and position. Exposes `toDevice(logicalX, logicalY)` = `round(v / scale)` and the
output geometry for cropping. Unit-tested (pure math, no side effects).

### capture/actions.ts
Primitives, each shelling `sg ydotool -c "YDOTOOL_SOCKET=… ydotool …"`:
- `moveTo(lx,ly)`, `click(lx,ly)` (move → settle → `click 0xC0`), `type(text)`,
  `key(combo)`, `wait(ms)`.
- Coordinates are **window-relative fractions** resolved against live window
  geometry at call time (see window.ts), then passed through `calibrate.toDevice`.
  This is what keeps clicks landing despite ydotool being coordinate-based.
- `--dry-run`: log intended commands without sending. `--probe`: move cursor to
  each resolved point (no click) to eyeball targeting.

### capture/window.ts
- Finds the window by class `Strikehub` via `hyprctl clients -j`.
- `placeOnHeadless()`: batch focus→move→fullscreen onto the headless output.
- `geometry()`: returns current `at`/`size` (logical) for relative-coordinate
  resolution and for `wf-recorder`/`grim` cropping.

### capture/driver.ts
- Sources devshell env; launches `./target/debug/strikehub` with
  `DISPLAY=:0 GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1
  STRIKE48_API_URL=https://plg.strike48.test MATRIX_TLS_INSECURE=1`.
- Waits for the window, places it on headless, fullscreens.
- For each scene in `flow.ts`: start `wf-recorder -o HEADLESS-N -g "<geom>" -f
  recordings/<scene>.mp4`, replay steps, SIGINT the recorder.
- Determinism guards: fixed viewport; explicit `wait()`s after nav/network;
  cursor parked off-frame between meaningful actions.
- Flags: `--scene <name>` (one scene), `--dry-run`, `--probe`.

### capture/flow.ts (the scenes)
~8 scenes for a whole-app tour (operator edits freely — it's data):
1. App launch → StrikeHub shell + connector rail (Sage UI)
2. OIDC sign-in via system browser (auth flow)
3. Connector rail — status indicators, switch connectors
4. Launch **Pick** → its UI loads in-window
5. Pick pentest flow — kick off a scan/action
6. Results / evidence view
7. Easy-mode ↔ Advanced toggle (reveals KubeStudio)
8. Outro card
Each scene: `{ name, steps: Action[], caption: string, kenBurns?: {from,to} }`.

### remotion/
- Root composition `1920×1080 @ 30fps`. A `<Series>` built from `manifest.json`
  (no hardcoded timing; durations from ffprobe).
- Per scene: `<OffthreadVideo>` clip + Sage-styled caption overlay animated with
  `spring()`; optional Ken Burns `scale`/`translate` for detail moments.
- Cross-dissolves between scenes via `@remotion/transitions`.
- Intro (logo → tagline) and outro cards as React (no capture).
- No audio track. `npx remotion render Reel out/strikehub-reel.mp4`.

## Error handling

- **stage.sh cleanup trap** runs on any exit (success, error, Ctrl-C): kill app,
  remove all HEADLESS-* outputs, reset eDP-1. No leaked outputs / shifted layout.
- **App crash / no window in 40s:** driver aborts the run with the captured log
  tail; does not silently produce empty clips.
- **Per-scene isolation:** a failed scene re-records alone via `--scene`; other
  clips untouched.
- **Network-timing variance** (live auth/connector responses): handled with
  generous `wait()`s and re-record-per-scene, not frame-perfect assumptions. This
  is the one thing capture cannot fully control; `log()` when a scene retries.
- **Connector auto-fetch 404s** (e.g. pick release download) are non-fatal and
  ignored — the sibling-workspace binary is used.

## Testing strategy

Pragmatic for a capture tool, not unit-test theater:
- `calibrate.ts` transform: unit-tested pure math (logical↔device).
- `actions.ts`: `--dry-run` verifies the command sequence without firing.
- Capture: `--scene <name>` runs one scene; `--probe` draws resolved click points
  (moves, no clicks) so targeting can be eyeballed before a real run.
- A first-run **smoke check** replicates the validated recipe: launch → assert a
  `Strikehub` window appears on the headless output within 40s → `grim` a frame →
  assert it's non-trivial (> size threshold, not solid background). This is the
  "is the environment sane" gate.
- Remotion: `npm run preview` (Remotion Studio) for visual verification pre-render.

## Multi-platform seam (future)

Same `flow.ts` runs per OS; clips land in `recordings/<os>/`; `manifest.json`
gains an `os` field; Remotion renders per-OS reels or an intercut montage. The
capture primitives (`actions`, `window`, `driver`) get per-OS backends behind the
same interface (macOS: screencapture + cliclick/AppleScript; Windows: interactive
session recording). Nothing in the Remotion stage changes.

## Open items (polish, not viability)

- Exclude the Hyprland top bar from capture (crop to window geometry via
  `wf-recorder -g`) so it never appears in footage.
- Decide caption visual style against the Sage design tokens (lower-third vs
  corner card).
- Supply intro/outro copy + logo asset.
