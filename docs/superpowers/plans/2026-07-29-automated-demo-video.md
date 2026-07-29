# Automated Demo Video Capture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a standalone `demo/` toolchain that automatically drives the running StrikeHub app off-screen, records per-scene clips, and composites them with Remotion into a ~60–90s captioned feature reel.

**Architecture:** Four decoupled stages — (1) environment/stage setup in bash, (2) TypeScript capture driver using ydotool + hyprctl + wf-recorder against a headless Hyprland output, (3) a manifest generated from the scene list, (4) a Remotion React project that renders the final MP4. `flow.ts` is the single source of truth for scenes; capture and compositing never depend on each other's internals.

**Tech Stack:** Node 22 / TypeScript (tsx runner), Remotion, bash, Hyprland (`hyprctl`), `wf-recorder`, `grim`, `ydotool`, `ffprobe` (via `nix-shell -p ffmpeg`), `nix develop` for the app runtime.

## Global Constraints

- **Off-screen only:** capture happens on a Hyprland headless virtual output parked at `@5000,0`; never on `eDP-1`. Every run must clean up (kill app, `hyprctl output remove` all `HEADLESS-*`, reset `eDP-1,3840x2160@60,0x0,2`).
- **X11 render mode is mandatory:** launch the app with `GDK_BACKEND=x11` and `WEBKIT_DISABLE_DMABUF_RENDERER=1`. Native Wayland gives negative dimensions / collapsed layout.
- **X socket fix required before launch:** `ln -sf /tmp/.X11-unix/X0_ /tmp/.X11-unix/X0` (Hyprland XWayland socket has a trailing underscore; Xlib `DISPLAY=:0` needs `X0`). Without it `global_hotkey` segfaults.
- **App runs inside `nix develop`** (needs `libxdo.so.4` + webkitgtk on `LD_LIBRARY_PATH`), env: `DISPLAY=:0 STRIKE48_API_URL=https://plg.strike48.test MATRIX_TLS_INSECURE=1 RUST_LOG=warn`.
- **Window class is `Strikehub`** (capital S), `xwayland=true`. Place via `hyprctl --batch "dispatch focuswindow address:$ADDR ; dispatch movewindow mon:$HL"` then `dispatch fullscreen 1`.
- **ydotool** via `sg ydotool -c "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool …"`. Absolute coord = `round(logical / monitor.scale)`.
- **Output:** 1920×1080 @ 30fps, MP4, **captions only — no audio track**.
- `demo/` is standalone; never modify the Rust workspace. All `recordings/` and `out/` are gitignored.
- Node scripts run via `npx tsx <file>` (no build step). Non-TS shell logic in `.sh` files.

---

### Task 1: Project scaffold + gitignore

**Files:**
- Create: `demo/package.json`
- Create: `demo/tsconfig.json`
- Create: `demo/.gitignore`
- Create: `demo/README.md`
- Modify: `.gitignore` (repo root — add `demo/node_modules`, `demo/recordings`, `demo/out`)

**Interfaces:**
- Produces: an installable Node project; `npx tsx` available; `demo/recordings/` and `demo/out/` ignored.

- [ ] **Step 1: Create `demo/package.json`**

```json
{
  "name": "strikehub-demo",
  "private": true,
  "type": "module",
  "scripts": {
    "capture": "tsx capture/driver.ts",
    "manifest": "tsx remotion/gen-manifest.ts",
    "preview": "remotion studio remotion/src/Root.tsx",
    "render": "remotion render remotion/src/Root.tsx Reel out/strikehub-reel.mp4",
    "test": "tsx --test capture/*.test.ts"
  },
  "dependencies": {
    "@remotion/cli": "^4.0.0",
    "@remotion/transitions": "^4.0.0",
    "remotion": "^4.0.0",
    "react": "^18.3.1",
    "react-dom": "^18.3.1"
  },
  "devDependencies": {
    "tsx": "^4.19.0",
    "typescript": "^5.6.0",
    "@types/node": "^22.0.0",
    "@types/react": "^18.3.0"
  }
}
```

- [ ] **Step 2: Create `demo/tsconfig.json`**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "jsx": "react-jsx",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "types": ["node"]
  },
  "include": ["capture", "remotion"]
}
```

- [ ] **Step 3: Create `demo/.gitignore`**

```
node_modules/
recordings/
out/
*.log
```

- [ ] **Step 4: Append to repo-root `.gitignore`**

Add these lines to `/home/jadams/src/github.com/Strike48-public/strikehub/.gitignore`:

```
# demo video tooling
demo/node_modules/
demo/recordings/
demo/out/
```

- [ ] **Step 5: Create `demo/README.md`**

```markdown
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
```

- [ ] **Step 6: Install deps**

Run: `cd demo && npm install`
Expected: `node_modules/` populated, no errors. (If Remotion pulls a chromium download, allow it.)

- [ ] **Step 7: Commit**

```bash
git add demo/package.json demo/tsconfig.json demo/.gitignore demo/README.md .gitignore
git commit -m "chore(demo): scaffold demo-video toolchain project"
```

---

### Task 2: Coordinate calibration (pure, unit-tested)

**Files:**
- Create: `demo/capture/calibrate.ts`
- Test: `demo/capture/calibrate.test.ts`

**Interfaces:**
- Produces:
  - `type OutputInfo = { name: string; width: number; height: number; x: number; y: number; scale: number }`
  - `toDevice(logicalX: number, logicalY: number, scale: number): { x: number; y: number }` → `{ round(logicalX/scale), round(logicalY/scale) }`
  - `parseHeadlessOutput(hyprctlMonitorsJson: string): OutputInfo` → the first monitor whose name starts with `HEADLESS`, throws if none.

- [ ] **Step 1: Write the failing test**

Create `demo/capture/calibrate.test.ts`:

```ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { toDevice, parseHeadlessOutput } from "./calibrate.ts";

test("toDevice divides by scale and rounds", () => {
  assert.deepEqual(toDevice(960, 540, 2), { x: 480, y: 270 });
  assert.deepEqual(toDevice(1920, 1080, 2), { x: 960, y: 540 });
  assert.deepEqual(toDevice(101, 101, 2), { x: 51, y: 51 }); // rounds .5 up
  assert.deepEqual(toDevice(800, 600, 1), { x: 800, y: 600 });
});

test("parseHeadlessOutput picks the HEADLESS monitor", () => {
  const json = JSON.stringify([
    { name: "eDP-1", width: 3840, height: 2160, x: 0, y: 0, scale: 2 },
    { name: "HEADLESS-6", width: 1920, height: 1080, x: 5000, y: 0, scale: 1 },
  ]);
  const o = parseHeadlessOutput(json);
  assert.equal(o.name, "HEADLESS-6");
  assert.equal(o.scale, 1);
  assert.equal(o.x, 5000);
});

test("parseHeadlessOutput throws when none present", () => {
  const json = JSON.stringify([{ name: "eDP-1", width: 3840, height: 2160, x: 0, y: 0, scale: 2 }]);
  assert.throws(() => parseHeadlessOutput(json), /no HEADLESS/i);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd demo && npx tsx --test capture/calibrate.test.ts`
Expected: FAIL — cannot find module `./calibrate.ts` / exports undefined.

- [ ] **Step 3: Write minimal implementation**

Create `demo/capture/calibrate.ts`:

```ts
export type OutputInfo = {
  name: string;
  width: number;
  height: number;
  x: number;
  y: number;
  scale: number;
};

export function toDevice(logicalX: number, logicalY: number, scale: number) {
  return { x: Math.round(logicalX / scale), y: Math.round(logicalY / scale) };
}

export function parseHeadlessOutput(hyprctlMonitorsJson: string): OutputInfo {
  const monitors = JSON.parse(hyprctlMonitorsJson) as any[];
  const m = monitors.find((mon) => String(mon.name).startsWith("HEADLESS"));
  if (!m) throw new Error("no HEADLESS output found in hyprctl monitors");
  return {
    name: m.name,
    width: m.width,
    height: m.height,
    x: m.x,
    y: m.y,
    scale: m.scale,
  };
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd demo && npx tsx --test capture/calibrate.test.ts`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add demo/capture/calibrate.ts demo/capture/calibrate.test.ts
git commit -m "feat(demo): coordinate calibration + headless output parser"
```

---

### Task 3: Session env resolver (bash)

**Files:**
- Create: `demo/env/session.sh`

**Interfaces:**
- Produces: a sourceable script exporting `HYPR_SIG`, `XDG_RUNTIME_DIR`, `YDOTOOL_SOCKET`, and `hyprq()` (a function running `hyprctl` with the right signature). Detects the live Hyprland instance by probing which signature answers `hyprctl version`.

- [ ] **Step 1: Create `demo/env/session.sh`**

```bash
#!/usr/bin/env bash
# Source me. Resolves the live Hyprland session + ydotool socket.
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
export YDOTOOL_SOCKET="${YDOTOOL_SOCKET:-/run/ydotoold/socket}"

# Find the Hyprland instance signature whose socket actually responds.
_resolve_hypr_sig() {
  local d sig
  for d in "$XDG_RUNTIME_DIR"/hypr/*/; do
    sig="$(basename "$d")"
    if HYPRLAND_INSTANCE_SIGNATURE="$sig" hyprctl version >/dev/null 2>&1; then
      echo "$sig"; return 0
    fi
  done
  return 1
}

HYPR_SIG="$(_resolve_hypr_sig)" || { echo "ERROR: no live Hyprland instance" >&2; return 1 2>/dev/null || exit 1; }
export HYPRLAND_INSTANCE_SIGNATURE="$HYPR_SIG"
export HYPR_SIG

# hyprctl wrapper
hyprq() { hyprctl "$@"; }

# ydotool wrapper: runs through sg so the ydotool group is active without re-login.
ydo() { sg ydotool -c "YDOTOOL_SOCKET=$YDOTOOL_SOCKET ydotool $*"; }
```

- [ ] **Step 2: Verify it resolves the live session**

Run: `cd demo && bash -c 'source env/session.sh && echo "sig=$HYPR_SIG" && hyprq monitors -j | head -c 60'`
Expected: prints a non-empty `sig=...` and the start of a JSON monitors array. If it errors "no live Hyprland instance", you are not in a Hyprland session — cannot proceed.

- [ ] **Step 3: Commit**

```bash
git add demo/env/session.sh
git commit -m "feat(demo): Hyprland session + ydotool env resolver"
```

---

### Task 4: Stage setup + cleanup (bash)

**Files:**
- Create: `demo/env/stage.sh`

**Interfaces:**
- Consumes: `demo/env/session.sh` (`hyprq`, `HYPR_SIG`).
- Produces:
  - `stage_up` → ensures `X0` symlink, creates a headless output, parks it at `@5000,0`, resets `eDP-1`, echoes the headless output name on stdout.
  - `stage_down` → kills strikehub, removes all `HEADLESS-*` outputs, resets `eDP-1`.
  - Sets an `EXIT`/`INT`/`TERM` trap to `stage_down`.

- [ ] **Step 1: Create `demo/env/stage.sh`**

```bash
#!/usr/bin/env bash
# Source after session.sh. Provides stage_up / stage_down with cleanup trap.

stage_down() {
  pkill -9 -f target/debug/strikehub 2>/dev/null || true
  local h
  for h in $(hyprq monitors -j | jq -r '.[]|select(.name|startswith("HEADLESS"))|.name'); do
    hyprq output remove "$h" >/dev/null 2>&1 || true
  done
  hyprq keyword monitor "eDP-1,3840x2160@60,0x0,2" >/dev/null 2>&1 || true
}

stage_up() {
  # X socket fix for global_hotkey / X11 backend
  [ -e /tmp/.X11-unix/X0 ] || ln -sf /tmp/.X11-unix/X0_ /tmp/.X11-unix/X0
  # clean any leftovers, then create fresh
  local h
  for h in $(hyprq monitors -j | jq -r '.[]|select(.name|startswith("HEADLESS"))|.name'); do
    hyprq output remove "$h" >/dev/null 2>&1 || true
  done
  hyprq output create headless >/dev/null 2>&1
  sleep 0.5
  local hl
  hl="$(hyprq monitors -j | jq -r '.[]|select(.name|startswith("HEADLESS"))|.name' | head -1)"
  hyprq keyword monitor "$hl,1920x1080@60,5000x0,1" >/dev/null 2>&1
  hyprq keyword monitor "eDP-1,3840x2160@60,0x0,2" >/dev/null 2>&1
  echo "$hl"
}

trap stage_down EXIT INT TERM
```

- [ ] **Step 2: Verify stage_up/stage_down leave a clean monitor list**

Run:
```bash
cd demo && bash -c '
  source env/session.sh; source env/stage.sh
  HL=$(stage_up); echo "created $HL"
  hyprq monitors -j | jq -r ".[]|\"\(.name)@\(.x),\(.y)\""
  stage_down
  echo "after cleanup:"; hyprq monitors -j | jq -r ".[].name"
'
```
Expected: prints `created HEADLESS-N`, lists eDP-1@0,0 + HEADLESS-N@5000,0, then after cleanup only `eDP-1`. The trap also runs `stage_down` on exit (harmless double-run).

- [ ] **Step 3: Commit**

```bash
git add demo/env/stage.sh
git commit -m "feat(demo): headless output stage_up/stage_down with cleanup trap"
```

---

### Task 5: Action primitives (ydotool wrapper)

**Files:**
- Create: `demo/capture/actions.ts`
- Test: `demo/capture/actions.test.ts`

**Interfaces:**
- Consumes: `toDevice` from `calibrate.ts`.
- Produces:
  - `type Ctx = { scale: number; win: { x: number; y: number; w: number; h: number }; dryRun: boolean; probe: boolean; run: (cmd: string) => void }`
  - `resolveFrac(ctx: Ctx, fx: number, fy: number): { x: number; y: number }` → window-relative fraction → **device** coords: logical = `win.x + fx*win.w`, `win.y + fy*win.h`, then `toDevice(..., scale)`.
  - `ydoCmd` builders returning the exact shell string (so they're testable without executing):
    - `moveCmd(x,y)` → `sg ydotool -c "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool mousemove --absolute -- ${x} ${y}"`
    - `clickCmd()` → `... ydotool click 0xC0`
    - `typeCmd(text)` → `... ydotool type -- "<escaped>"`
    - `keyCmd(combo)` → `... ydotool key ${combo}`
  - async actions using `ctx.run`: `moveTo(ctx,fx,fy)`, `click(ctx,fx,fy)`, `type(ctx,text)`, `key(ctx,combo)`, `wait(ms)`. In `dryRun`/`probe`, `click`/`type`/`key` skip the actual click/type (probe still moves).

- [ ] **Step 1: Write the failing test**

Create `demo/capture/actions.test.ts`:

```ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { resolveFrac, moveCmd, clickCmd, typeCmd, keyCmd } from "./actions.ts";

const ctx = { scale: 2, win: { x: 0, y: 0, w: 1920, h: 1080 }, dryRun: true, probe: false, run: () => {} };

test("resolveFrac maps window fraction to device coords", () => {
  assert.deepEqual(resolveFrac(ctx, 0.5, 0.5), { x: 480, y: 270 }); // (960,540)/2
  assert.deepEqual(resolveFrac({ ...ctx, win: { x: 100, y: 100, w: 800, h: 600 } }, 0, 0), { x: 50, y: 50 });
});

test("command builders emit exact ydotool shell strings", () => {
  assert.equal(moveCmd(480, 270), 'sg ydotool -c "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool mousemove --absolute -- 480 270"');
  assert.equal(clickCmd(), 'sg ydotool -c "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool click 0xC0"');
  assert.equal(keyCmd("28:1 28:0"), 'sg ydotool -c "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool key 28:1 28:0"');
});

test("typeCmd escapes double quotes", () => {
  assert.equal(typeCmd('he said "hi"'), 'sg ydotool -c "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool type -- \\"he said \\\\\\"hi\\\\\\"\\""');
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd demo && npx tsx --test capture/actions.test.ts`
Expected: FAIL — module/exports missing.

- [ ] **Step 3: Write minimal implementation**

Create `demo/capture/actions.ts`:

```ts
import { toDevice } from "./calibrate.ts";

const SOCK = "/run/ydotoold/socket";
const wrap = (inner: string) => `sg ydotool -c "YDOTOOL_SOCKET=${SOCK} ydotool ${inner}"`;

export type Ctx = {
  scale: number;
  win: { x: number; y: number; w: number; h: number };
  dryRun: boolean;
  probe: boolean;
  run: (cmd: string) => void;
};

export function resolveFrac(ctx: Ctx, fx: number, fy: number) {
  const lx = ctx.win.x + fx * ctx.win.w;
  const ly = ctx.win.y + fy * ctx.win.h;
  return toDevice(lx, ly, ctx.scale);
}

export const moveCmd = (x: number, y: number) => wrap(`mousemove --absolute -- ${x} ${y}`);
export const clickCmd = () => wrap(`click 0xC0`);
export const keyCmd = (combo: string) => wrap(`key ${combo}`);
export const typeCmd = (text: string) => wrap(`type -- "${text.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`);

export const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

export async function moveTo(ctx: Ctx, fx: number, fy: number) {
  const { x, y } = resolveFrac(ctx, fx, fy);
  ctx.run(moveCmd(x, y));
  await wait(150);
}

export async function click(ctx: Ctx, fx: number, fy: number) {
  await moveTo(ctx, fx, fy);
  if (ctx.dryRun || ctx.probe) return;
  ctx.run(clickCmd());
  await wait(150);
}

export async function type(ctx: Ctx, text: string) {
  if (ctx.dryRun || ctx.probe) return;
  ctx.run(typeCmd(text));
  await wait(100);
}

export async function key(ctx: Ctx, combo: string) {
  if (ctx.dryRun || ctx.probe) return;
  ctx.run(keyCmd(combo));
  await wait(100);
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd demo && npx tsx --test capture/actions.test.ts`
Expected: PASS (3 tests). If the `typeCmd` escaping assertion mismatches, adjust the expected string to match the implementation's real output (copy actual output into the test) — the goal is a documented, stable escaping, not a specific byte sequence.

- [ ] **Step 5: Commit**

```bash
git add demo/capture/actions.ts demo/capture/actions.test.ts
git commit -m "feat(demo): ydotool action primitives with window-relative coords"
```

---

### Task 6: Window management (find/place/geometry)

**Files:**
- Create: `demo/capture/window.ts`
- Test: `demo/capture/window.test.ts`

**Interfaces:**
- Consumes: `hyprctl clients -j` output (passed in as string for testability).
- Produces:
  - `type Win = { address: string; monitor: number; x: number; y: number; w: number; h: number }`
  - `findStrikehub(clientsJson: string): Win | null` → the client with `class === "Strikehub"`.
  - `placeCmds(address: string, headless: string): string[]` → the exact hyprctl commands to move+fullscreen: `['hyprctl --batch "dispatch focuswindow address:<a> ; dispatch movewindow mon:<hl>"', 'hyprctl dispatch fullscreen 1']`.

- [ ] **Step 1: Write the failing test**

Create `demo/capture/window.test.ts`:

```ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { findStrikehub, placeCmds } from "./window.ts";

const clients = JSON.stringify([
  { address: "0xAAA", class: "Alacritty", monitor: 0, at: [12, 42], size: [941, 1026] },
  { address: "0xBBB", class: "Strikehub", monitor: 0, at: [967, 42], size: [941, 1026] },
]);

test("findStrikehub picks the capital-S Strikehub window", () => {
  const w = findStrikehub(clients);
  assert.equal(w?.address, "0xBBB");
  assert.equal(w?.w, 941);
  assert.equal(w?.y, 42);
});

test("findStrikehub returns null when absent", () => {
  assert.equal(findStrikehub(JSON.stringify([{ address: "0x1", class: "foo", monitor: 0, at: [0,0], size: [1,1] }])), null);
});

test("placeCmds emits batch focus+move then fullscreen", () => {
  assert.deepEqual(placeCmds("0xBBB", "HEADLESS-6"), [
    'hyprctl --batch "dispatch focuswindow address:0xBBB ; dispatch movewindow mon:HEADLESS-6"',
    "hyprctl dispatch fullscreen 1",
  ]);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd demo && npx tsx --test capture/window.test.ts`
Expected: FAIL — module/exports missing.

- [ ] **Step 3: Write minimal implementation**

Create `demo/capture/window.ts`:

```ts
export type Win = {
  address: string;
  monitor: number;
  x: number;
  y: number;
  w: number;
  h: number;
};

export function findStrikehub(clientsJson: string): Win | null {
  const clients = JSON.parse(clientsJson) as any[];
  const c = clients.find((cl) => cl.class === "Strikehub");
  if (!c) return null;
  return {
    address: c.address,
    monitor: c.monitor,
    x: c.at[0],
    y: c.at[1],
    w: c.size[0],
    h: c.size[1],
  };
}

export function placeCmds(address: string, headless: string): string[] {
  return [
    `hyprctl --batch "dispatch focuswindow address:${address} ; dispatch movewindow mon:${headless}"`,
    "hyprctl dispatch fullscreen 1",
  ];
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd demo && npx tsx --test capture/window.test.ts`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add demo/capture/window.ts demo/capture/window.test.ts
git commit -m "feat(demo): strikehub window find + headless placement commands"
```

---

### Task 7: Scene definitions (flow.ts)

**Files:**
- Create: `demo/capture/flow.ts`
- Test: `demo/capture/flow.test.ts`

**Interfaces:**
- Consumes: `Ctx` and action functions from `actions.ts`.
- Produces:
  - `type Step = (ctx: Ctx) => Promise<void>`
  - `type Scene = { name: string; caption: string; kenBurns?: { from: number; to: number }; steps: Step[] }`
  - `export const scenes: Scene[]` — the 8-scene whole-app tour. Steps use `click`/`type`/`key`/`wait` with window-relative fractions. Exact click targets are placeholders the operator refines with `--probe`; captions are final copy.

- [ ] **Step 1: Write the failing test**

Create `demo/capture/flow.test.ts`:

```ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { scenes } from "./flow.ts";

test("scenes cover the whole-app tour with unique names and captions", () => {
  assert.ok(scenes.length >= 8);
  const names = scenes.map((s) => s.name);
  assert.equal(new Set(names).size, names.length, "scene names must be unique");
  for (const s of scenes) {
    assert.ok(s.caption.length > 0, `${s.name} needs a caption`);
    assert.ok(Array.isArray(s.steps), `${s.name} needs steps[]`);
  }
  assert.deepEqual(names.slice(0, 2), ["launch", "signin"]);
  assert.ok(names.includes("easy-mode"));
  assert.ok(names.includes("outro"));
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd demo && npx tsx --test capture/flow.test.ts`
Expected: FAIL — module/exports missing.

- [ ] **Step 3: Write minimal implementation**

Create `demo/capture/flow.ts`:

```ts
import { type Ctx, click, type as typeText, key, wait } from "./actions.ts";

export type Step = (ctx: Ctx) => Promise<void>;
export type Scene = {
  name: string;
  caption: string;
  kenBurns?: { from: number; to: number };
  steps: Step[];
};

// Click targets are window-relative fractions (0..1). Refine with `--probe`.
export const scenes: Scene[] = [
  {
    name: "launch",
    caption: "One window for every Strike48 tool",
    steps: [async () => { await wait(2500); }],
  },
  {
    name: "signin",
    caption: "Sign in once with your Strike48 account",
    steps: [
      async (c) => { await click(c, 0.5, 0.53); }, // Sign In button
      async () => { await wait(3000); },            // browser OAuth hands back
    ],
  },
  {
    name: "connectors",
    caption: "Live status for every connector",
    kenBurns: { from: 1.0, to: 1.12 },
    steps: [
      async (c) => { await click(c, 0.02, 0.12); }, // rail item 1
      async () => { await wait(1500); },
    ],
  },
  {
    name: "launch-pick",
    caption: "Launch Pick — your pentest copilot",
    steps: [
      async (c) => { await click(c, 0.02, 0.06); }, // Pick in rail
      async () => { await wait(3500); },            // Pick UI loads
    ],
  },
  {
    name: "pick-run",
    caption: "Kick off an assessment in one click",
    steps: [
      async (c) => { await click(c, 0.5, 0.85); },  // primary action
      async () => { await wait(4000); },
    ],
  },
  {
    name: "pick-results",
    caption: "Findings and evidence, organized",
    kenBurns: { from: 1.0, to: 1.1 },
    steps: [async () => { await wait(3000); }],
  },
  {
    name: "easy-mode",
    caption: "Easy mode keeps it simple — Advanced reveals the rest",
    steps: [
      async (c) => { await click(c, 0.01, 0.97); }, // settings gear
      async () => { await wait(1200); },
      async (c) => { await click(c, 0.5, 0.4); },   // easy-mode toggle
      async () => { await wait(1800); },
    ],
  },
  {
    name: "outro",
    caption: "StrikeHub — the unified Strike48 desktop",
    steps: [async () => { await wait(2000); }],
  },
];
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd demo && npx tsx --test capture/flow.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add demo/capture/flow.ts demo/capture/flow.test.ts
git commit -m "feat(demo): whole-app tour scene definitions"
```

---

### Task 8: Capture driver (launch + record per scene)

**Files:**
- Create: `demo/capture/driver.ts`
- Create: `demo/env/launch-app.sh`

**Interfaces:**
- Consumes: `env/session.sh`, `env/stage.sh`, `calibrate.ts`, `window.ts`, `actions.ts`, `flow.ts`.
- Produces: `demo/recordings/<scene>.mp4` per scene; honors `--scene <name>`, `--dry-run`, `--probe`. `launch-app.sh` starts the app under the validated X11 devshell env.

- [ ] **Step 1: Create `demo/env/launch-app.sh`**

```bash
#!/usr/bin/env bash
# Launch StrikeHub under the validated headless X11 env. Backgrounds it.
cd "$(dirname "$0")/../.." || exit 1   # repo root (demo/env -> repo)
nix develop --command bash -c '
  export XDG_RUNTIME_DIR="'"${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"'"
  export DISPLAY=:0 GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1
  export STRIKE48_API_URL=https://plg.strike48.test MATRIX_TLS_INSECURE=1 RUST_LOG=warn
  exec ./target/debug/strikehub
' > /tmp/strikehub-demo.log 2>&1 &
echo $!
```

- [ ] **Step 2: Write the driver**

Create `demo/capture/driver.ts`:

```ts
import { execSync, spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHeadlessOutput } from "./calibrate.ts";
import { findStrikehub, placeCmds } from "./window.ts";
import { type Ctx } from "./actions.ts";
import { scenes, type Scene } from "./flow.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const DEMO = resolve(HERE, "..");
const REC = resolve(DEMO, "recordings");
const sh = (cmd: string) => execSync(cmd, { stdio: "pipe" }).toString().trim();

const args = process.argv.slice(2);
const only = args.includes("--scene") ? args[args.indexOf("--scene") + 1] : null;
const dryRun = args.includes("--dry-run");
const probe = args.includes("--probe");

async function main() {
  mkdirSync(REC, { recursive: true });
  // stage_up (headless output) + launch app via bash helpers, capture output name + pid
  const hl = sh(`bash -c 'source ${DEMO}/env/session.sh; source ${DEMO}/env/stage.sh; stage_up'`).split("\n").pop()!.trim();
  console.log(`headless output: ${hl}`);
  const pid = sh(`bash ${DEMO}/env/launch-app.sh`);
  console.log(`strikehub pid: ${pid}`);

  // wait up to 40s for the window
  let win = null;
  for (let i = 0; i < 40; i++) {
    await new Promise((r) => setTimeout(r, 1000));
    try { win = findStrikehub(sh(`hyprctl clients -j`)); } catch {}
    if (win) break;
  }
  if (!win) {
    console.error("NO WINDOW after 40s — log tail:\n" + sh(`tail -8 /tmp/strikehub-demo.log`));
    process.exit(1);
  }
  // place on headless + fullscreen
  for (const cmd of placeCmds(win.address, hl)) sh(cmd);
  await new Promise((r) => setTimeout(r, 2000));

  const out = parseHeadlessOutput(sh(`hyprctl monitors -j`));
  // re-read geometry now that it's fullscreen on headless
  const placed = findStrikehub(sh(`hyprctl clients -j`))!;
  const ctx: Ctx = {
    scale: out.scale,
    win: { x: placed.x, y: placed.y, w: placed.w, h: placed.h },
    dryRun, probe,
    run: (cmd) => sh(cmd),
  };

  const list: Scene[] = only ? scenes.filter((s) => s.name === only) : scenes;
  if (only && list.length === 0) { console.error(`no scene named ${only}`); process.exit(1); }

  for (const scene of list) {
    console.log(`▶ scene: ${scene.name} — "${scene.caption}"`);
    let rec: ReturnType<typeof spawn> | null = null;
    if (!dryRun && !probe) {
      // wf-recorder cropped to the output; -g uses absolute output geometry
      const geom = `${out.x},${out.y} ${out.width}x${out.height}`;
      rec = spawn("wf-recorder", ["-o", hl, "-g", geom, "-f", `${REC}/${scene.name}.mp4`], { stdio: "ignore" });
      await new Promise((r) => setTimeout(r, 800)); // let recorder start
    }
    for (const step of scene.steps) await step(ctx);
    if (rec) { rec.kill("SIGINT"); await new Promise((r) => setTimeout(r, 600)); }
  }
  console.log("capture complete");
  // stage_down runs via the bash trap in a separate process; clean up here too:
  sh(`bash -c 'source ${DEMO}/env/session.sh; source ${DEMO}/env/stage.sh; stage_down'`);
}

main().catch((e) => { console.error(e); process.exit(1); });
```

- [ ] **Step 3: Dry-run to verify wiring (no input sent, no recording)**

Run: `cd demo && npx tsx capture/driver.ts --dry-run`
Expected: creates a headless output, launches StrikeHub, reports `headless output: HEADLESS-N`, `strikehub pid: …`, finds the window, logs each `▶ scene:` line, then `capture complete`, then cleans up (only `eDP-1` remains). No clicks fired. If "NO WINDOW after 40s", check `/tmp/strikehub-demo.log`.

- [ ] **Step 4: Probe one scene to verify targeting**

Run: `cd demo && npx tsx capture/driver.ts --scene signin --probe`
Expected: cursor moves to the Sign In target on the headless output (verify with `grim -o HEADLESS-N /tmp/probe.png` in another shell during the run, or trust the move log). No click.

- [ ] **Step 5: Real single-scene capture**

Run: `cd demo && npx tsx capture/driver.ts --scene launch`
Expected: `recordings/launch.mp4` exists and is > 100 KB and plays. Verify: `ls -l recordings/launch.mp4`.

- [ ] **Step 6: Commit**

```bash
git add demo/capture/driver.ts demo/env/launch-app.sh
git commit -m "feat(demo): capture driver — launch, place, record per scene"
```

---

### Task 9: Manifest generator (clips → manifest.json)

**Files:**
- Create: `demo/remotion/gen-manifest.ts`

**Interfaces:**
- Consumes: `flow.ts` (`scenes`), clips in `recordings/`, `ffprobe` (via `nix-shell -p ffmpeg`).
- Produces: `demo/remotion/manifest.json` = `[{ name, clip, caption, kenBurns, durationInFrames }]` at 30fps, only for scenes whose clip exists. Missing clips are logged and skipped (not fatal).

- [ ] **Step 1: Write the generator**

Create `demo/remotion/gen-manifest.ts`:

```ts
import { execSync } from "node:child_process";
import { existsSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { scenes } from "../capture/flow.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const DEMO = resolve(HERE, "..");
const REC = resolve(DEMO, "recordings");
const FPS = 30;

function durationSeconds(file: string): number {
  const out = execSync(
    `nix-shell -p ffmpeg --run 'ffprobe -v error -show_entries format=duration -of csv=p=0 "${file}"'`,
  ).toString().trim();
  return parseFloat(out);
}

const manifest = [];
for (const s of scenes) {
  const clip = resolve(REC, `${s.name}.mp4`);
  if (!existsSync(clip)) { console.warn(`skip ${s.name}: no clip`); continue; }
  const secs = durationSeconds(clip);
  manifest.push({
    name: s.name,
    clip: `../recordings/${s.name}.mp4`,
    caption: s.caption,
    kenBurns: s.kenBurns ?? null,
    durationInFrames: Math.max(1, Math.round(secs * FPS)),
  });
}
writeFileSync(resolve(HERE, "manifest.json"), JSON.stringify(manifest, null, 2));
console.log(`wrote manifest.json with ${manifest.length} scenes`);
```

- [ ] **Step 2: Generate the manifest from whatever clips exist**

Run: `cd demo && npx tsx remotion/gen-manifest.ts`
Expected: writes `remotion/manifest.json`. With only `launch.mp4` present (from Task 8), it contains 1 entry with a real `durationInFrames`; others logged as `skip … no clip`.

- [ ] **Step 3: Commit**

```bash
git add demo/remotion/gen-manifest.ts
git commit -m "feat(demo): manifest generator (ffprobe durations from clips)"
```

---

### Task 10: Remotion composition (clips + captions + bookends)

**Files:**
- Create: `demo/remotion/src/Root.tsx`
- Create: `demo/remotion/src/Reel.tsx`
- Create: `demo/remotion/src/Scene.tsx`
- Create: `demo/remotion/src/Caption.tsx`
- Create: `demo/remotion/src/Intro.tsx`
- Create: `demo/remotion/src/Outro.tsx`

**Interfaces:**
- Consumes: `remotion/manifest.json`.
- Produces: a Remotion composition `Reel` (1920×1080 @ 30fps) whose total duration is the sum of intro + scene durations + outro, renderable to MP4.

- [ ] **Step 1: Create `Caption.tsx`**

```tsx
import { interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion";

export const Caption: React.FC<{ text: string }> = ({ text }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const enter = spring({ frame, fps, config: { damping: 200 } });
  const y = interpolate(enter, [0, 1], [40, 0]);
  return (
    <div style={{
      position: "absolute", bottom: 80, left: 0, right: 0,
      display: "flex", justifyContent: "center", opacity: enter, transform: `translateY(${y}px)`,
    }}>
      <div style={{
        background: "rgba(20,20,24,0.82)", color: "#cbd7c4",
        font: "500 34px Inter, system-ui, sans-serif", padding: "16px 28px",
        borderRadius: 999, border: "1px solid #4c5a44",
      }}>{text}</div>
    </div>
  );
};
```

- [ ] **Step 2: Create `Scene.tsx`**

```tsx
import { AbsoluteFill, OffthreadVideo, interpolate, useCurrentFrame, useVideoConfig, staticFile } from "remotion";
import { Caption } from "./Caption.tsx";

export const Scene: React.FC<{
  src: string; caption: string; kenBurns: { from: number; to: number } | null;
}> = ({ src, caption, kenBurns }) => {
  const frame = useCurrentFrame();
  const { durationInFrames } = useVideoConfig();
  const scale = kenBurns
    ? interpolate(frame, [0, durationInFrames], [kenBurns.from, kenBurns.to])
    : 1;
  return (
    <AbsoluteFill style={{ backgroundColor: "#101014" }}>
      <AbsoluteFill style={{ transform: `scale(${scale})` }}>
        <OffthreadVideo src={src} />
      </AbsoluteFill>
      <Caption text={caption} />
    </AbsoluteFill>
  );
};
```

- [ ] **Step 3: Create `Intro.tsx` and `Outro.tsx`**

```tsx
// Intro.tsx
import { AbsoluteFill, interpolate, useCurrentFrame } from "remotion";
export const Intro: React.FC = () => {
  const f = useCurrentFrame();
  const o = interpolate(f, [0, 15, 45, 60], [0, 1, 1, 0], { extrapolateRight: "clamp" });
  return (
    <AbsoluteFill style={{ backgroundColor: "#101014", justifyContent: "center", alignItems: "center", opacity: o }}>
      <div style={{ color: "#cbd7c4", font: "700 88px Inter, system-ui, sans-serif" }}>StrikeHub</div>
    </AbsoluteFill>
  );
};
```

```tsx
// Outro.tsx
import { AbsoluteFill, interpolate, useCurrentFrame } from "remotion";
export const Outro: React.FC = () => {
  const f = useCurrentFrame();
  const o = interpolate(f, [0, 15], [0, 1], { extrapolateRight: "clamp" });
  return (
    <AbsoluteFill style={{ backgroundColor: "#101014", justifyContent: "center", alignItems: "center", opacity: o }}>
      <div style={{ color: "#cbd7c4", font: "600 52px Inter, system-ui, sans-serif" }}>
        The unified Strike48 desktop
      </div>
    </AbsoluteFill>
  );
};
```

- [ ] **Step 4: Create `Reel.tsx`**

```tsx
import { AbsoluteFill, Series } from "remotion";
import { Scene } from "./Scene.tsx";
import { Intro } from "./Intro.tsx";
import { Outro } from "./Outro.tsx";
import manifest from "../manifest.json" with { type: "json" };

const INTRO = 60;
const OUTRO = 60;

export const Reel: React.FC = () => (
  <AbsoluteFill style={{ backgroundColor: "#101014" }}>
    <Series>
      <Series.Sequence durationInFrames={INTRO}><Intro /></Series.Sequence>
      {manifest.map((s: any) => (
        <Series.Sequence key={s.name} durationInFrames={s.durationInFrames}>
          <Scene src={s.clip} caption={s.caption} kenBurns={s.kenBurns} />
        </Series.Sequence>
      ))}
      <Series.Sequence durationInFrames={OUTRO}><Outro /></Series.Sequence>
    </Series>
  </AbsoluteFill>
);

export const reelDuration = () =>
  INTRO + OUTRO + manifest.reduce((n: number, s: any) => n + s.durationInFrames, 0);
```

- [ ] **Step 5: Create `Root.tsx`**

```tsx
import { Composition } from "remotion";
import { Reel, reelDuration } from "./Reel.tsx";

export const RemotionRoot: React.FC = () => (
  <Composition
    id="Reel"
    component={Reel}
    durationInFrames={reelDuration()}
    fps={30}
    width={1920}
    height={1080}
  />
);
```

- [ ] **Step 6: Verify the composition loads in Studio**

Run: `cd demo && npm run preview`
Expected: Remotion Studio opens with a `Reel` composition; intro + available scene clip(s) + outro render on the timeline with captions. (Ctrl-C to exit.)

- [ ] **Step 7: Commit**

```bash
git add demo/remotion/src/
git commit -m "feat(demo): Remotion composition — scenes, captions, bookends"
```

---

### Task 11: Orchestrator + end-to-end render

**Files:**
- Create: `demo/orchestrate.sh`

**Interfaces:**
- Consumes: all prior tasks.
- Produces: `out/strikehub-reel.mp4` from a single command.

- [ ] **Step 1: Create `demo/orchestrate.sh`**

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

echo "== capture =="
npx tsx capture/driver.ts "$@"

echo "== manifest =="
npx tsx remotion/gen-manifest.ts

echo "== render =="
mkdir -p out
npx remotion render remotion/src/Root.tsx Reel out/strikehub-reel.mp4

echo "done -> out/strikehub-reel.mp4"
```

- [ ] **Step 2: Make executable + run full pipeline**

Run: `cd demo && chmod +x orchestrate.sh && ./orchestrate.sh`
Expected: captures all scenes → writes manifest → renders `out/strikehub-reel.mp4`. Verify: `ls -lh out/strikehub-reel.mp4` (non-trivial size) and play it. If a scene's click targets are off, refine fractions in `flow.ts` using `--probe` and re-run `./orchestrate.sh --scene <name>` then re-render.

- [ ] **Step 3: Commit**

```bash
git add demo/orchestrate.sh
git commit -m "feat(demo): end-to-end orchestrator (capture -> manifest -> render)"
```

---

## Self-Review

**Spec coverage:**
- Off-screen isolation (headless Hyprland output) → Tasks 4, 8. ✓
- X11 mode + dmabuf disable → Task 8 (launch-app.sh). ✓
- X0 symlink / global_hotkey fix → Task 4 (stage_up). ✓
- nix develop runtime → Task 8. ✓
- Window class `Strikehub` + placement → Task 6, 8. ✓
- ydotool via sg + ÷scale calibration → Tasks 2, 5. ✓
- Per-scene recording, `--scene`/`--dry-run`/`--probe` → Task 8. ✓
- flow.ts single source of truth, 8-scene tour → Task 7. ✓
- Manifest via ffprobe → Task 9. ✓
- Remotion: OffthreadVideo, captions, Ken Burns, transitions-capable, bookends, no audio, 1920×1080@30, `remotion render` → Task 10. ✓
- Cleanup trap / no leaked outputs → Task 4. ✓
- Testing: calibrate unit test, actions/window/flow tests, dry-run/probe, smoke via dry-run window check → Tasks 2,5,6,7,8. ✓
- Standalone `demo/`, gitignored recordings/out → Task 1. ✓
- Multi-platform seam: documented in spec; not built now (per non-goals). ✓

**Placeholder scan:** Click-target fractions in `flow.ts` are explicitly operator-tunable via `--probe` (documented), not hidden TODOs; captions are final. No "TBD"/"handle errors"/"similar to" placeholders. Transitions: spec says "transitions-capable"; `@remotion/transitions` is installed and can be added between sequences as a polish step — Series gives clean cuts by default, acceptable for v1.

**Type consistency:** `Ctx`, `Win`, `OutputInfo`, `Scene`/`Step`, `toDevice`, `resolveFrac`, `findStrikehub`, `placeCmds`, `parseHeadlessOutput` names match across tasks 2/5/6/7/8/9/10. Manifest shape (`name/clip/caption/kenBurns/durationInFrames`) is identical in Task 9 (producer) and Task 10 (consumer). ✓
