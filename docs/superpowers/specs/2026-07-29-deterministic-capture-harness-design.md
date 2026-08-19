# Deterministic Demo-Capture Harness — Design

**Date:** 2026-07-29
**Status:** Approved design, ready for implementation planning
**Author:** Josh Adams (with Claude)

## Goal

Replace the hand-driven ("vibey") capture step of the demo-video pipeline with a
**deterministic, Stagehand-style harness**: natural-language step instructions
resolved to concrete click targets by a vision LLM **once**, cached to disk, and
replayed with zero LLM calls on subsequent runs. The harness drives the real
StrikeHub app off-screen, records per-scene clips, and hands them to the existing
Remotion pipeline — producing the demo reel reproducibly (`orchestrate.sh` → same
video), with a human touch only for OAuth login.

The reel showcases (real, authenticated, against `plg.strike48.test`): network
scan with live tool calls → generated report → shareable report doc (scrolled) →
turn off StrikeHub Easy Mode to reveal KubeStudio (real k8s dashboard) → turn off
Pick Easy Mode to reveal the expert pentest interface.

## Non-goals

- Not re-capturing footage on every run when the cache is valid (replay is the norm).
- Not eliminating the human OAuth step (in-memory token, system-browser flow).
- Not driving via Playwright/CDP or the strikehub-server web build — we drive the
  **native WebKitGTK window** via ydotool (Stagehand-the-library is not usable here;
  we implement its *pattern*).
- Not a general-purpose UI automation framework — scoped to this demo flow.

## What we reuse vs. replace

**Reuse (already built + working):**
- `env/session.sh`, `env/stage.sh`, `env/launch-app.sh` — Hyprland session resolve,
  headless output stage_up/down, X11-mode app launch.
- `capture/calibrate.ts`, `capture/window.ts` — monitor scale + window find/place.
- `remotion/*` — composition, captions, intro/outro, manifest, gen-manifest.
- `recordings/build-scenes.sh` — normalize/trim/speed.
- `orchestrate.sh` — end-to-end capture→manifest→render.

**Replace (the vibey parts):**
- `capture/driver.ts` → `capture/runner.ts` (deterministic flow executor).
- `capture/flow.ts` → `capture/flow.json` (versioned scene+step spec, data not code).

**Add:**
- `capture/agent.ts` — Claude vision client: `act()` + `verify()`.
- `capture/cache.ts` — load/save `cache/actions.json`.
- `capture/screenshot.ts` — grim wrapper → PNG buffer.
- `cache/actions.json` — committed resolved coords (makes replay deterministic).

## Key validated facts (from prior capture work — must hold in the harness)

- **Coordinate formula:** headless output parked contiguous at global x=1920
  (`hyprctl keyword monitor "HEADLESS-N,1920x1080@60,1920x0,1"`). ydotool absolute
  space maps `global_logical = ydotool * 2`. To click *screenshot* pixel (sx, sy) on
  the headless output: **`ydotool_x = (1920 + sx) / 2`, `ydotool_y = sy / 2`**.
  (The 1920 offset is essential — omitting it clicks the real eDP screen.) The
  existing `capture/actions.ts` coordinate handling MUST be corrected to this;
  its current formula (`toDevice(logical/scale)` without the 1920 offset) is the
  bug that made all left-side clicks land off-window.
- **ydotool:** use the **system** daemon socket `/run/ydotoold/socket`; operator must
  be in the `ydotool` group; call ydotool directly with
  `YDOTOOL_SOCKET=/run/ydotoold/socket` (no `sg`). WebKit acts on synthetic clicks
  when coords are right and the window is focused.
- **grim:** needs `WAYLAND_DISPLAY=wayland-1` **and** `-o HEADLESS-N`.
- **App launch:** `nix develop` + `DISPLAY=:0 GDK_BACKEND=x11
  WEBKIT_DISABLE_DMABUF_RENDERER=1` (X11 mode mandatory; native Wayland gives
  collapsed layout). X socket fix: `ln -sf /tmp/.X11-unix/X0_ /tmp/.X11-unix/X0`
  (else `global_hotkey` segfaults).
- **Focus:** the StrikeHub window must be **focused for the whole recording** and the
  runner must not issue focus-stealing commands mid-record.
- **Color:** wf-recorder output is `yuvj420p`/full-range but loses matrix tags; the
  reel came out mistagged `bt470bg`. Every ffmpeg re-encode and the Remotion render
  MUST force **full-range bt709** (`-color_range pc -colorspace bt709
  -color_primaries bt709 -color_trc bt709`, and `scale=out_range=full:
  out_color_matrix=bt709` where scaling).
- **wf-recorder CFR:** output is variable-framerate; `setpts` speed-up does nothing
  until normalized to CFR first (`-r 30 -vsync cfr`), then `setpts=(1/SPEED)*PTS -r 30`.

## Architecture

```
demo/
  capture/
    agent.ts          # Claude vision: act(instr,png)->{x,y}; verify(expect,png)->bool
    cache.ts          # cache/actions.json load/save (stepId -> {x,y})
    screenshot.ts     # grim -o HEADLESS-N (WAYLAND_DISPLAY=wayland-1) -> Buffer
    actions.ts        # REUSE+FIX: ydotool click/move/type/key/scroll; correct coord formula
    window.ts         # REUSE: findStrikehub, placeCmds
    calibrate.ts      # REUSE: monitor scale/geometry
    flow.json         # NEW: scenes[] with steps[] + trim[] (source of truth)
    runner.ts         # NEW: deterministic executor (replaces driver.ts)
  cache/
    actions.json      # committed: resolved coords for deterministic replay
    failures/         # gitignored: screenshots saved on verify timeout
  env/*.sh            # REUSE (+ dim-disable + focus assert in stage.sh)
  recordings/
    build-scenes.sh   # REUSE+EXTEND: bt709 tags; motion-aware/per-scene trim from flow.json
  remotion/*          # REUSE+EXTEND: TransitionSeries (fades/dissolves); bt709 render
  orchestrate.sh      # REUSE: capture -> build-scenes -> manifest -> render
```

### flow.json format

```jsonc
{
  "version": 1,
  "output": { "headlessName": "HEADLESS", "width": 1920, "height": 1080, "fps": 30 },
  "auth": { "signInStep": { "act": "click the Sign In button" },
            "readyVerify": "authenticated home with the connector rail is visible" },
  "scenes": [
    {
      "id": "scan",
      "caption": "Ask Pick to scan your network — it runs the tools for you",
      "record": true,
      "steps": [
        { "id": "scan.click_scan",
          "act": "click the 'Scan My Network' button",
          "verify": "the agent is executing tool calls (Phase 1 / device_info visible)",
          "timeoutMs": 20000 },
        { "id": "scan.wait_report",
          "verify": "a Network Discovery Report with host counts / findings is visible",
          "timeoutMs": 240000 }
      ],
      // Editorial: which parts of the raw clip to show, at what speed.
      // Anchors: "start", "end", "verify:<stepId>" (timestamp when that verify passed),
      // or "+Ns"/"-Ns" relative. Favor movement over static settled frames.
      "trim": [
        { "from": "verify:scan.click_scan", "to": "+42s", "speed": 1.8 },
        { "from": "end-12s", "to": "end", "speed": 1.3 }   // last 12s of streaming movement
      ]
    },
    {
      "id": "doc",
      "caption": "Every scan becomes a shareable report",
      "record": true,
      "steps": [
        { "id": "doc.open", "act": "click the Network Discovery Report in DOCUMENTS FROM THIS CHAT",
          "verify": "the report document is open (Executive Summary visible)", "timeoutMs": 15000 },
        { "id": "doc.scroll", "scroll": { "direction": "down", "amount": "page", "repeat": 4, "settleMs": 900 } }
      ],
      "trim": [ { "from": "verify:doc.open", "to": "end", "speed": 1.0 } ]
    }
    // ... easymode (toggle -> KubeStudio), pickmode (toggle -> expert) scenes
  ]
}
```

- **act step:** `act` (NL instruction) → click target, cached by `id`. Optional
  `verify` confirms the click landed.
- **wait step:** only `verify` → poll until satisfied or `timeoutMs`.
- **scroll step:** `scroll` config → runner scrolls the focused view (reveals long
  docs as motion instead of static hold).
- **trim:** per-scene editorial segments (favor movement); consumed by build-scenes.

### agent.ts (Claude vision)

- `act(instruction, pngBuffer, outputDims) -> { x, y, reasoning }`. Anthropic API,
  vision model (Sonnet-class), forced structured output (tool schema) returning
  click point in **screenshot pixels**. Key from `ANTHROPIC_API_KEY`.
- `verify(expectation, pngBuffer) -> { satisfied: boolean, reasoning }`.
- Only called on cache miss / `--refresh` (for `act`) or live each poll (for `verify`).

### cache.ts

- `cache/actions.json`: `{ [stepId]: { x, y, resolvedAt, instruction } }`.
- `get(stepId)`, `set(stepId, coords)`, persisted as pretty JSON (committed).
- Replay with a populated cache makes **zero `act()` LLM calls**. `verify()` polls
  live each run (chosen for timing robustness — clicks deterministic, advancement live).

### runner.ts (deterministic executor)

1. **Setup:** source `env/session.sh`; `stage_up` (create+park headless output,
   X0 symlink, **disable dim, assert focus**); `launch-app.sh`; wait for `Strikehub`
   window; place fullscreen on headless output.
2. **Auth gate:** run `auth.signInStep` (act "click Sign In"); print a clear
   **PAUSE** prompt; poll `auth.readyVerify` until the operator completes OAuth in
   the browser; then proceed unattended.
3. **Per scene** (`record:true`): assert window focus; start `wf-recorder -o
   HEADLESS-N -g "<geom>"`; execute steps in order (act→resolve(cache|agent)→
   validate-in-window→click; wait→poll verify; scroll→scroll); record the wall-clock
   timestamp each verify passes (for `trim` anchors → written to
   `recordings/<scene>.timings.json`); SIGINT the recorder.
4. **Teardown (trap, always):** kill app; remove all HEADLESS-*; restore monitor
   layout and dim settings.

Flags: `--refresh` (re-resolve all act steps), `--scene <id>`, `--from <id>`,
`--no-record` (dry drive for tuning/first-resolve).

### build-scenes.sh (extended)

- Normalize each raw `recordings/<scene>.mp4` to CFR 30fps.
- Apply the scene's `trim` segments (resolving anchors via
  `recordings/<scene>.timings.json`), concatenating segments at their speeds.
- Force **full-range bt709** on every re-encode.
- Output `recordings/scenes/<scene>[_n].mp4` + emit durations for the manifest.

### remotion (extended)

- `Reel.tsx` uses `@remotion/transitions` `<TransitionSeries>` with a
  cross-dissolve/fade (~15-frame) between every scene and the bookends — no hard cuts.
- Ken Burns retained per-scene (from manifest) so inherently-static shots still move.
- Render forces bt709 (post-encode step in `orchestrate.sh` if the encoder won't tag).

## Data flow

`flow.json` → `runner.ts` (drives app, records) → `recordings/<scene>.mp4` +
`<scene>.timings.json` → `build-scenes.sh` (trim/speed/color) →
`recordings/scenes/*.mp4` → `gen-manifest.ts` → `manifest.json` → Remotion render →
`out/strikehub-reel.mp4`. Cache (`cache/actions.json`) makes the runner step
deterministic on replay.

## Error handling

- **act() off-window coords:** runner validates the point lies within the window
  rect; one retry with a sharper prompt; then fail the step loudly (never click nothing).
- **verify() timeout:** save last screenshot to `cache/failures/<stepId>.png`, fail
  the scene (no infinite hang).
- **New step id, no cache, no --refresh:** resolve live and append to cache (adding a
  step doesn't force full re-resolve).
- **App crash / no window in 40s:** abort with `/tmp/strikehub-demo.log` tail (GL +
  global_hotkey workarounds are in launch-app.sh).
- **scroll doesn't move the view:** ydotool wheel is unreliable; the scroll primitive
  has a fallback chain (wheel → focus+PageDown key → scrollbar click-drag) and the
  runner verifies the view changed (screenshot diff) or logs that it didn't.
- **Teardown always runs** (trap) — no leaked headless outputs / shifted screen.

## Testing

- **Unit (no LLM/app):** calibrate coord math incl. the `(1920+sx)/2` formula;
  cache.ts load/save/roundtrip; flow.json schema validation; actions.ts command-string
  builders (click/move/type/key/scroll) incl. shell-safety.
- **Integration (live, gated):** `runner.ts --no-record` dry drive — asserts window
  appears, one `act()` resolves to in-window coords, one `verify()` returns true.
  Not mocked; exercises Claude live. Requires the operator/session (Hyprland +
  ydotool group + ANTHROPIC_API_KEY) — skipped in CI, run locally.
- **Smoke:** the existing "window appears + non-trivial screenshot" gate.

## Open items (polish, not viability)

- Motion-aware auto-trim (mpdecimate) as an optional enhancement to explicit
  `trim` segments — deferred; explicit trim ships first.
- Scroll primitive reliability against WebKitGTK is the one real implementation risk;
  the fallback chain + verify mitigates it.
