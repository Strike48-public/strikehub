# Deterministic Demo-Capture Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hand-driven capture step of the demo-video pipeline with a deterministic Stagehand-style harness: natural-language step instructions resolved to click targets by Claude vision **once**, cached to disk, and replayed with zero LLM calls.

**Architecture:** A `flow.json` scene/step spec drives `runner.ts`, which launches StrikeHub off-screen (headless Hyprland output, X11 mode), records per-scene clips with `wf-recorder`, and drives the UI via `ydotool`. Click targets come from a disk cache (`cache/actions.json`) populated by `agent.ts` (Claude vision `act()`); step advancement uses live `agent.verify()` polling. Existing Remotion compositing, `build-scenes.sh`, and `env/*.sh` are reused/extended (bt709 color, scene transitions).

**Tech Stack:** Node 22 / TypeScript (`tsx` runner), `@anthropic-ai/sdk` (Claude vision), Remotion + `@remotion/transitions`, bash, `ydotool`/`hyprctl`/`grim`/`wf-recorder`, `ffmpeg`/`ffprobe` via `nix-shell -p ffmpeg`.

## Global Constraints

- **Coordinate formula (verified):** headless output parked contiguous at global x=1920 (`hyprctl keyword monitor "HEADLESS-N,1920x1080@60,1920x0,1"`). ydotool absolute maps `global_logical = ydotool * 2`. To click *screenshot* pixel (sx, sy): `ydotool_x = round((1920 + sx) / 2)`, `ydotool_y = round(sy / 2)`. The 1920 offset is mandatory — omitting it clicks the real screen.
- **ydotool:** system daemon socket `/run/ydotoold/socket`; operator in `ydotool` group; call ydotool directly with `YDOTOOL_SOCKET=/run/ydotoold/socket` (no `sg`).
- **grim:** needs `WAYLAND_DISPLAY=wayland-1` AND `-o HEADLESS-N`.
- **App launch:** inside `nix develop`, env `DISPLAY=:0 GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 STRIKE48_API_URL=https://plg.strike48.test MATRIX_TLS_INSECURE=1`. X11 mode mandatory. X socket fix: `ln -sf /tmp/.X11-unix/X0_ /tmp/.X11-unix/X0` (else global_hotkey segfaults).
- **Focus:** StrikeHub window must stay focused during each recording; runner must not run focus-stealing commands mid-record.
- **Color:** every ffmpeg re-encode and the Remotion render force full-range bt709 (`-color_range pc -colorspace bt709 -color_primaries bt709 -color_trc bt709`; when scaling, `scale=...:out_range=full:out_color_matrix=bt709`).
- **CFR before speed:** normalize wf-recorder output to CFR 30fps (`-r 30 -vsync cfr`) BEFORE any `setpts` speed change, else setpts is a no-op.
- **LLM:** Claude via `@anthropic-ai/sdk`, model `claude-opus-4-8`, `ANTHROPIC_API_KEY` from env. Vision via base64 PNG image blocks. Structured output via forced tool use (`tool_choice: {type:"tool", name:...}`).
- **Cache determinism:** `act()` results cached by step id in `cache/actions.json`; replay makes zero `act()` LLM calls. `verify()` polls live each run (not cached).
- **Window class is `Strikehub`** (capital S), `xwayland=true`.
- **Output:** 1920×1080 @ 30fps MP4, captions only, NO audio. Scene transitions (fades/dissolves), not hard cuts.
- `demo/` is standalone; never modify the Rust workspace. `recordings/`, `out/`, `cache/failures/` gitignored.

---

### Task 1: Add SDK dependency + cache/failures gitignore

**Files:**
- Modify: `demo/package.json` (add `@anthropic-ai/sdk`)
- Modify: `demo/.gitignore` (add `cache/failures/`)

**Interfaces:**
- Produces: `@anthropic-ai/sdk` importable; `cache/failures/` ignored.

- [ ] **Step 1: Add the dependency to `demo/package.json`**

In the `"dependencies"` object add `"@anthropic-ai/sdk": "^0.68.0"` (alongside the existing remotion/react entries). Exact key/value:

```json
"@anthropic-ai/sdk": "^0.68.0"
```

- [ ] **Step 2: Append to `demo/.gitignore`**

Add this line (the file already ignores node_modules/recordings/out):

```
cache/failures/
```

- [ ] **Step 3: Install**

Run: `cd demo && npm install`
Expected: `@anthropic-ai/sdk` appears in `node_modules/@anthropic-ai/sdk`, no errors.

- [ ] **Step 4: Verify import resolves**

Run: `cd demo && npx tsx -e "import Anthropic from '@anthropic-ai/sdk'; console.log(typeof Anthropic)"`
Expected: prints `function`.

- [ ] **Step 5: Commit**

```bash
git add demo/package.json demo/package-lock.json demo/.gitignore
git commit -m "chore(demo): add @anthropic-ai/sdk for the capture harness"
```

---

### Task 2: Coordinate transform — the (1920+sx)/2 formula

**Files:**
- Modify: `demo/capture/calibrate.ts`
- Test: `demo/capture/calibrate.test.ts`

**Interfaces:**
- Consumes: existing `OutputInfo` and `parseHeadlessOutput` in calibrate.ts (keep them).
- Produces: `screenToYdotool(sx: number, sy: number, out: OutputInfo): { x: number; y: number }` returning `{ round((out.x + sx)/out.scale), round(sy/out.scale) }`. (For the parked headless output, `out.x` is the global x offset — 1920 — and `out.scale` is 2 on this box; the formula generalizes both.)

- [ ] **Step 1: Write the failing test**

Append to `demo/capture/calibrate.test.ts`:

```ts
import { screenToYdotool } from "./calibrate.ts";

test("screenToYdotool applies (offsetX + sx)/scale and sy/scale", () => {
  const out = { name: "HEADLESS-1", width: 1920, height: 1080, x: 1920, y: 0, scale: 2 };
  // screenshot center (960,540) on a 1920-offset scale-2 output -> global (2880,540) -> ydotool (1440,270)
  assert.deepEqual(screenToYdotool(960, 540, out), { x: 1440, y: 270 });
  // screenshot origin (0,0) -> global (1920,0) -> ydotool (960,0)
  assert.deepEqual(screenToYdotool(0, 0, out), { x: 960, y: 0 });
  // rounding: (1920+1)/2 = 960.5 -> 961
  assert.deepEqual(screenToYdotool(1, 0, out), { x: 961, y: 0 });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd demo && npx tsx --test capture/calibrate.test.ts`
Expected: FAIL — `screenToYdotool` is not exported.

- [ ] **Step 3: Add the implementation**

Append to `demo/capture/calibrate.ts`:

```ts
/**
 * Convert a screenshot pixel (sx, sy) on the given output to ydotool absolute
 * coordinates. ydotool's absolute space is the compositor's device space, so
 * global_logical = ydotool * scale — hence ydotool = (global_logical) / scale,
 * where global_logical_x = out.x (offset of the output) + sx.
 */
export function screenToYdotool(sx: number, sy: number, out: OutputInfo) {
  return {
    x: Math.round((out.x + sx) / out.scale),
    y: Math.round(sy / out.scale),
  };
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd demo && npx tsx --test capture/calibrate.test.ts`
Expected: PASS (all calibrate tests, including the existing ones).

- [ ] **Step 5: Commit**

```bash
git add demo/capture/calibrate.ts demo/capture/calibrate.test.ts
git commit -m "feat(demo): screenToYdotool coordinate transform ((offset+sx)/scale)"
```

---

### Task 3: Action primitives — click/move/type/key/scroll via ydotool system socket

**Files:**
- Create: `demo/capture/actions.ts` (overwrite the old vibey one)
- Test: `demo/capture/actions.test.ts` (overwrite)

**Interfaces:**
- Consumes: `screenToYdotool` (calibrate.ts), `OutputInfo`.
- Produces:
  - `const SOCK = "/run/ydotoold/socket"`
  - `moveCmd(x,y): string` → `YDOTOOL_SOCKET=/run/ydotoold/socket ydotool mousemove --absolute -- ${x} ${y}`
  - `clickCmd(): string` → `... ydotool click 0xC0`
  - `keyCmd(codes: string): string` → `... ydotool key ${codes}`
  - `typeCmd(text: string): string` → `... ydotool type -- ${shQuote(text)}` (single-quote-safe)
  - `scrollCmd(dy: number): string` → `... ydotool mousemove --wheel -- 0 ${dy}` (negative = down in ydotool wheel convention; runner passes the sign)
  - `shQuote(s): string` → POSIX single-quote escaping
  - These builders return the inner command (no `sg`); the runner prefixes `YDOTOOL_SOCKET=` via env when executing.

Note: builders are pure (return command strings) so they're unit-testable; the runner executes them with `execSync`.

- [ ] **Step 1: Write the failing test**

Create `demo/capture/actions.test.ts`:

```ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { moveCmd, clickCmd, keyCmd, typeCmd, scrollCmd, shQuote } from "./actions.ts";

test("shQuote wraps in single quotes and escapes embedded single quotes", () => {
  assert.equal(shQuote("hello"), "'hello'");
  assert.equal(shQuote("it's"), "'it'\\''s'");
});

test("command builders emit the expected ydotool strings", () => {
  assert.equal(moveCmd(480, 270), "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool mousemove --absolute -- 480 270");
  assert.equal(clickCmd(), "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool click 0xC0");
  assert.equal(keyCmd("1:1 1:0"), "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool key 1:1 1:0");
  assert.equal(scrollCmd(-15), "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool mousemove --wheel -- 0 -15");
});

test("typeCmd single-quotes the payload safely", () => {
  assert.equal(typeCmd("hi there"), "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool type -- 'hi there'");
  assert.equal(typeCmd("it's"), "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool type -- 'it'\\''s'");
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd demo && npx tsx --test capture/actions.test.ts`
Expected: FAIL — module/exports missing.

- [ ] **Step 3: Write the implementation**

Create `demo/capture/actions.ts`:

```ts
export const SOCK = "/run/ydotoold/socket";

/** POSIX single-quote escaping: close, escaped-quote, reopen. */
export function shQuote(s: string): string {
  return `'${s.replace(/'/g, `'\\''`)}'`;
}

const wrap = (inner: string) => `YDOTOOL_SOCKET=${SOCK} ydotool ${inner}`;

export const moveCmd = (x: number, y: number) => wrap(`mousemove --absolute -- ${x} ${y}`);
export const clickCmd = () => wrap(`click 0xC0`);
export const keyCmd = (codes: string) => wrap(`key ${codes}`);
export const typeCmd = (text: string) => wrap(`type -- ${shQuote(text)}`);
/** dy negative = scroll down (ydotool wheel convention). */
export const scrollCmd = (dy: number) => wrap(`mousemove --wheel -- 0 ${dy}`);
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd demo && npx tsx --test capture/actions.test.ts`
Expected: PASS (3 tests). If `typeCmd`/`shQuote` expected strings mismatch the real output, capture the actual output and paste it into the test — the goal is documented, stable single-quote escaping.

- [ ] **Step 5: Commit**

```bash
git add demo/capture/actions.ts demo/capture/actions.test.ts
git commit -m "feat(demo): ydotool action command builders (system socket, scroll)"
```

---

### Task 4: Cache — load/save resolved coordinates

**Files:**
- Create: `demo/capture/cache.ts`
- Test: `demo/capture/cache.test.ts`

**Interfaces:**
- Produces:
  - `type CachedAction = { x: number; y: number; instruction: string; resolvedAt: string }`
  - `type ActionCache = Record<string, CachedAction>`
  - `loadCache(path: string): ActionCache` — returns `{}` if the file doesn't exist.
  - `saveCache(path: string, cache: ActionCache): void` — writes pretty JSON (2-space), creating parent dirs.
  - `getCached(cache, stepId): CachedAction | undefined`
  - `setCached(cache, stepId, entry): void` (mutates the object)

- [ ] **Step 1: Write the failing test**

Create `demo/capture/cache.test.ts`:

```ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadCache, saveCache, getCached, setCached } from "./cache.ts";

test("loadCache returns empty object when file is absent", () => {
  const dir = mkdtempSync(join(tmpdir(), "cachetest-"));
  assert.deepEqual(loadCache(join(dir, "nope.json")), {});
  rmSync(dir, { recursive: true });
});

test("save then load roundtrips, and get/set work", () => {
  const dir = mkdtempSync(join(tmpdir(), "cachetest-"));
  const p = join(dir, "sub", "actions.json"); // parent dir does not exist yet
  const cache = {};
  setCached(cache, "scan.click", { x: 1440, y: 76, instruction: "click Scan", resolvedAt: "2026-07-29T00:00:00Z" });
  saveCache(p, cache);
  const loaded = loadCache(p);
  assert.equal(getCached(loaded, "scan.click")?.x, 1440);
  assert.equal(getCached(loaded, "missing"), undefined);
  rmSync(dir, { recursive: true });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd demo && npx tsx --test capture/cache.test.ts`
Expected: FAIL — module/exports missing.

- [ ] **Step 3: Write the implementation**

Create `demo/capture/cache.ts`:

```ts
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

export type CachedAction = { x: number; y: number; instruction: string; resolvedAt: string };
export type ActionCache = Record<string, CachedAction>;

export function loadCache(path: string): ActionCache {
  if (!existsSync(path)) return {};
  return JSON.parse(readFileSync(path, "utf8")) as ActionCache;
}

export function saveCache(path: string, cache: ActionCache): void {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, JSON.stringify(cache, null, 2) + "\n");
}

export function getCached(cache: ActionCache, stepId: string): CachedAction | undefined {
  return cache[stepId];
}

export function setCached(cache: ActionCache, stepId: string, entry: CachedAction): void {
  cache[stepId] = entry;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd demo && npx tsx --test capture/cache.test.ts`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add demo/capture/cache.ts demo/capture/cache.test.ts
git commit -m "feat(demo): action-coordinate cache (load/save/get/set)"
```

---

### Task 5: flow.json schema + loader/validator

**Files:**
- Create: `demo/capture/flow.ts` (overwrite the old vibey one)
- Create: `demo/capture/flow.json`
- Test: `demo/capture/flow.test.ts` (overwrite)

**Interfaces:**
- Produces:
  - `type Step = { id: string; act?: string; verify?: string; scroll?: { dy: number; repeat: number; settleMs: number }; timeoutMs?: number }`
  - `type TrimSeg = { from: string; to: string; speed: number }`
  - `type Scene = { id: string; caption: string; record: boolean; kenBurns?: {from:number;to:number}; steps: Step[]; trim?: TrimSeg[] }`
  - `type Flow = { version: number; output: { headlessName: string; width: number; height: number; fps: number }; auth: { signInStep: Step; readyVerify: string }; scenes: Scene[] }`
  - `loadFlow(path: string): Flow` — reads + `validateFlow`.
  - `validateFlow(obj: unknown): Flow` — throws with a clear message on: missing version/output/auth/scenes; duplicate scene ids; duplicate step ids (global); a step with neither `act` nor `verify` nor `scroll`.

- [ ] **Step 1: Write the failing test**

Create `demo/capture/flow.test.ts`:

```ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { validateFlow, loadFlow } from "./flow.ts";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));

test("validateFlow rejects duplicate step ids", () => {
  const bad = {
    version: 1, output: { headlessName: "HEADLESS", width: 1920, height: 1080, fps: 30 },
    auth: { signInStep: { id: "auth.signin", act: "click Sign In" }, readyVerify: "home visible" },
    scenes: [{ id: "a", caption: "c", record: true, steps: [
      { id: "dup", act: "x", verify: "y" }, { id: "dup", verify: "z" },
    ] }],
  };
  assert.throws(() => validateFlow(bad), /duplicate step id/i);
});

test("validateFlow rejects a step with no act/verify/scroll", () => {
  const bad = {
    version: 1, output: { headlessName: "HEADLESS", width: 1920, height: 1080, fps: 30 },
    auth: { signInStep: { id: "auth.signin", act: "click Sign In" }, readyVerify: "home visible" },
    scenes: [{ id: "a", caption: "c", record: true, steps: [{ id: "empty" }] }],
  };
  assert.throws(() => validateFlow(bad), /must have act, verify, or scroll/i);
});

test("the shipped flow.json is valid and has the 5 real scenes", () => {
  const flow = loadFlow(join(HERE, "flow.json"));
  const ids = flow.scenes.map((s) => s.id);
  for (const want of ["scan", "doc", "share", "easymode", "pickmode"]) {
    assert.ok(ids.includes(want), `missing scene ${want}`);
  }
  assert.ok(flow.auth.signInStep.act);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd demo && npx tsx --test capture/flow.test.ts`
Expected: FAIL — module/flow.json missing.

- [ ] **Step 3: Write `demo/capture/flow.ts`**

```ts
import { readFileSync } from "node:fs";

export type Step = {
  id: string;
  act?: string;
  verify?: string;
  scroll?: { dy: number; repeat: number; settleMs: number };
  timeoutMs?: number;
};
export type TrimSeg = { from: string; to: string; speed: number };
export type Scene = {
  id: string;
  caption: string;
  record: boolean;
  kenBurns?: { from: number; to: number };
  steps: Step[];
  trim?: TrimSeg[];
};
export type Flow = {
  version: number;
  output: { headlessName: string; width: number; height: number; fps: number };
  auth: { signInStep: Step; readyVerify: string };
  scenes: Scene[];
};

export function validateFlow(obj: unknown): Flow {
  const f = obj as Flow;
  if (!f || typeof f !== "object") throw new Error("flow: not an object");
  if (typeof f.version !== "number") throw new Error("flow: missing version");
  if (!f.output?.headlessName) throw new Error("flow: missing output.headlessName");
  if (!f.auth?.signInStep?.act) throw new Error("flow: missing auth.signInStep.act");
  if (!Array.isArray(f.scenes) || f.scenes.length === 0) throw new Error("flow: no scenes");
  const sceneIds = new Set<string>();
  const stepIds = new Set<string>();
  for (const sc of f.scenes) {
    if (sceneIds.has(sc.id)) throw new Error(`flow: duplicate scene id ${sc.id}`);
    sceneIds.add(sc.id);
    for (const st of sc.steps) {
      if (stepIds.has(st.id)) throw new Error(`flow: duplicate step id ${st.id}`);
      stepIds.add(st.id);
      if (!st.act && !st.verify && !st.scroll)
        throw new Error(`flow: step ${st.id} must have act, verify, or scroll`);
    }
  }
  return f;
}

export function loadFlow(path: string): Flow {
  return validateFlow(JSON.parse(readFileSync(path, "utf8")));
}
```

- [ ] **Step 4: Write `demo/capture/flow.json`** (the 5 real scenes; captions final, act instructions natural-language, timeouts generous for the scan)

```json
{
  "version": 1,
  "output": { "headlessName": "HEADLESS", "width": 1920, "height": 1080, "fps": 30 },
  "auth": {
    "signInStep": { "id": "auth.signin", "act": "click the 'Sign In' button", "verify": "an OAuth login browser has opened (app shows a signing-in / waiting state)", "timeoutMs": 20000 },
    "readyVerify": "the authenticated Pick home screen with a 'Scan My Network' button and a connector rail on the left is visible"
  },
  "scenes": [
    {
      "id": "scan",
      "caption": "Ask Pick to scan your network — it runs the tools for you",
      "record": true,
      "steps": [
        { "id": "scan.click_scan", "act": "click the large 'Scan My Network' button at the top", "verify": "the agent is executing tool calls (a Phase / device_info step with a success badge is visible)", "timeoutMs": 25000 },
        { "id": "scan.wait_report", "verify": "a Network Discovery Report summary with host counts and security findings is visible", "timeoutMs": 300000 }
      ],
      "trim": [
        { "from": "verify:scan.click_scan", "to": "+42s", "speed": 1.8 },
        { "from": "end-12s", "to": "end", "speed": 1.3 }
      ]
    },
    {
      "id": "doc",
      "caption": "Every scan becomes a shareable report",
      "record": true,
      "kenBurns": { "from": 1.0, "to": 1.06 },
      "steps": [
        { "id": "doc.open", "act": "click the 'Network Discovery Report' entry under 'DOCUMENTS FROM THIS CHAT' near the bottom", "verify": "the report document is open showing an Executive Summary heading", "timeoutMs": 15000 },
        { "id": "doc.scroll", "scroll": { "dy": -15, "repeat": 5, "settleMs": 900 } }
      ],
      "trim": [ { "from": "verify:doc.open", "to": "end", "speed": 1.0 } ]
    },
    {
      "id": "share",
      "caption": "Share it in one click",
      "record": true,
      "steps": [
        { "id": "share.click", "act": "click the share icon in the top-right corner of the report header", "verify": "a share menu or share affordance is shown", "timeoutMs": 10000 }
      ],
      "trim": [ { "from": "start", "to": "end", "speed": 1.0 } ]
    },
    {
      "id": "easymode",
      "caption": "Turn off Easy Mode to reveal KubeStudio",
      "record": true,
      "steps": [
        { "id": "easymode.settings", "act": "click the settings gear icon at the bottom of the far-left rail", "verify": "the Settings screen with an 'Easy mode' toggle is visible", "timeoutMs": 12000 },
        { "id": "easymode.toggle_off", "act": "click the 'Easy mode' toggle to turn it OFF", "verify": "both a KubeStudio card and a Pick card are shown (advanced connectors revealed)", "timeoutMs": 12000 },
        { "id": "easymode.open_kube", "act": "click the KubeStudio card", "verify": "the KubeStudio Kubernetes dashboard (Cluster Overview with Nodes/Pods/Workloads) is visible", "timeoutMs": 30000 }
      ],
      "trim": [ { "from": "verify:easymode.toggle_off", "to": "end", "speed": 1.2 } ]
    },
    {
      "id": "pickmode",
      "caption": "Pick has an Easy Mode too — flip it off for the expert view",
      "record": true,
      "steps": [
        { "id": "pickmode.to_pick", "act": "click the Pick (shield) icon in the far-left connector rail", "verify": "the Pick home screen is visible", "timeoutMs": 12000 },
        { "id": "pickmode.menu", "act": "click the hamburger menu icon at the top-left next to the 'Pick' title", "verify": "a slide-out menu with 'New chat', 'Reports', and 'Settings' is visible", "timeoutMs": 10000 },
        { "id": "pickmode.settings", "act": "click 'Settings' in the slide-out menu", "verify": "the Pick Settings screen with an 'Easy Mode' toggle is visible", "timeoutMs": 10000 },
        { "id": "pickmode.toggle_off", "act": "click the 'Easy Mode' toggle to turn it off", "verify": "the full expert Pentest dashboard with a Tools/Shell/Chat nav and Quick Actions grid is visible", "timeoutMs": 20000 }
      ],
      "trim": [ { "from": "verify:pickmode.toggle_off", "to": "end", "speed": 1.2 } ]
    }
  ]
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd demo && npx tsx --test capture/flow.test.ts`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add demo/capture/flow.ts demo/capture/flow.json demo/capture/flow.test.ts
git commit -m "feat(demo): flow.json spec + validating loader (5 real scenes)"
```

---

### Task 6: window helpers — find/place/geometry/focus

**Files:**
- Create: `demo/capture/window.ts` (overwrite the old one)
- Test: `demo/capture/window.test.ts` (overwrite)

**Interfaces:**
- Produces:
  - `type Win = { address: string; monitor: number; x: number; y: number; w: number; h: number; class: string }`
  - `findStrikehub(clientsJson: string): Win | null` — client with `class === "Strikehub"`.
  - `placeCmds(address: string, headless: string): string[]` → `['hyprctl --batch "dispatch focuswindow address:<a> ; dispatch movewindow mon:<hl>"', 'hyprctl dispatch fullscreen 1']`.
  - `focusCmd(address: string): string` → `hyprctl dispatch focuswindow address:<a>`.
  - `activeIsStrikehub(activeJson: string): boolean` → true if `JSON.parse(activeJson).class === "Strikehub"`.

- [ ] **Step 1: Write the failing test**

Create `demo/capture/window.test.ts`:

```ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { findStrikehub, placeCmds, focusCmd, activeIsStrikehub } from "./window.ts";

const clients = JSON.stringify([
  { address: "0xAAA", class: "Alacritty", monitor: 0, at: [12, 42], size: [900, 1000] },
  { address: "0xBBB", class: "Strikehub", monitor: 1, at: [1932, 42], size: [1896, 1026] },
]);

test("findStrikehub picks the capital-S window and maps at/size", () => {
  const w = findStrikehub(clients);
  assert.equal(w?.address, "0xBBB");
  assert.equal(w?.x, 1932);
  assert.equal(w?.w, 1896);
  assert.equal(w?.monitor, 1);
});

test("findStrikehub returns null when absent", () => {
  assert.equal(findStrikehub(JSON.stringify([{ address: "0x1", class: "x", monitor: 0, at: [0,0], size: [1,1] }])), null);
});

test("placeCmds and focusCmd emit exact strings", () => {
  assert.deepEqual(placeCmds("0xBBB", "HEADLESS-6"), [
    'hyprctl --batch "dispatch focuswindow address:0xBBB ; dispatch movewindow mon:HEADLESS-6"',
    "hyprctl dispatch fullscreen 1",
  ]);
  assert.equal(focusCmd("0xBBB"), "hyprctl dispatch focuswindow address:0xBBB");
});

test("activeIsStrikehub checks the active window class", () => {
  assert.equal(activeIsStrikehub(JSON.stringify({ class: "Strikehub" })), true);
  assert.equal(activeIsStrikehub(JSON.stringify({ class: "Alacritty" })), false);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd demo && npx tsx --test capture/window.test.ts`
Expected: FAIL — module/exports missing.

- [ ] **Step 3: Write the implementation**

Create `demo/capture/window.ts`:

```ts
export type Win = {
  address: string; monitor: number; x: number; y: number; w: number; h: number; class: string;
};

export function findStrikehub(clientsJson: string): Win | null {
  const clients = JSON.parse(clientsJson) as any[];
  const c = clients.find((cl) => cl.class === "Strikehub");
  if (!c) return null;
  return { address: c.address, monitor: c.monitor, x: c.at[0], y: c.at[1], w: c.size[0], h: c.size[1], class: c.class };
}

export function placeCmds(address: string, headless: string): string[] {
  return [
    `hyprctl --batch "dispatch focuswindow address:${address} ; dispatch movewindow mon:${headless}"`,
    "hyprctl dispatch fullscreen 1",
  ];
}

export function focusCmd(address: string): string {
  return `hyprctl dispatch focuswindow address:${address}`;
}

export function activeIsStrikehub(activeJson: string): boolean {
  try { return (JSON.parse(activeJson) as any).class === "Strikehub"; } catch { return false; }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd demo && npx tsx --test capture/window.test.ts`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add demo/capture/window.ts demo/capture/window.test.ts
git commit -m "feat(demo): window find/place/focus helpers"
```

---

### Task 7: env scripts — session, stage (dim off + X0 symlink), launch

**Files:**
- Modify: `demo/env/session.sh` (switch ydotool to system socket; add grim helper)
- Modify: `demo/env/stage.sh` (park headless at 1920,0; disable dim; X0 symlink; cleanup restores dim + eDP-1)
- Modify: `demo/env/launch-app.sh` (X11 env already present — verify/keep)

**Interfaces:**
- Produces (bash): `stage_up` echoes the headless output name; `stage_down` full cleanup; `shot <out.png>` grabs the headless output; `YDOTOOL_SOCKET` exported to the system socket.

- [ ] **Step 1: Update `demo/env/session.sh`**

Replace its body with:

```bash
#!/usr/bin/env bash
# Source me. Resolves the live Hyprland session + system ydotool socket + grim helper.
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
export YDOTOOL_SOCKET="${YDOTOOL_SOCKET:-/run/ydotoold/socket}"

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
export HYPRLAND_INSTANCE_SIGNATURE="$HYPR_SIG" HYPR_SIG

# Resolve the real Hyprland wayland display (grim needs it).
export WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-1}"

hyprq() { hyprctl "$@"; }
# grim the headless output: shot <headless-name> <out.png>
shot() { WAYLAND_DISPLAY="$WAYLAND_DISPLAY" grim -o "$1" "$2"; }
```

- [ ] **Step 2: Update `demo/env/stage.sh`**

```bash
#!/usr/bin/env bash
# Source after session.sh. stage_up/stage_down with cleanup trap.
# Disables Hyprland dim during capture and restores it after.

stage_down() {
  pkill -9 -f target/debug/strikehub 2>/dev/null || true
  pkill -9 wf-recorder 2>/dev/null || true
  local h
  for h in $(hyprq monitors -j | jq -r '.[]|select(.name|startswith("HEADLESS"))|.name'); do
    hyprq output remove "$h" >/dev/null 2>&1 || true
  done
  hyprq keyword monitor "eDP-1,3840x2160@60,0x0,2" >/dev/null 2>&1 || true
  # restore dim defaults
  hyprq keyword decoration:dim_inactive false >/dev/null 2>&1 || true
}

stage_up() {
  [ -e /tmp/.X11-unix/X0 ] || ln -sf /tmp/.X11-unix/X0_ /tmp/.X11-unix/X0
  local h
  for h in $(hyprq monitors -j | jq -r '.[]|select(.name|startswith("HEADLESS"))|.name'); do
    hyprq output remove "$h" >/dev/null 2>&1 || true
  done
  # ensure no inactive-window dim during capture
  hyprq keyword decoration:dim_inactive false >/dev/null 2>&1
  hyprq output create headless >/dev/null 2>&1
  sleep 0.5
  local hl
  hl="$(hyprq monitors -j | jq -r '.[]|select(.name|startswith("HEADLESS"))|.name' | head -1)"
  hyprq keyword monitor "$hl,1920x1080@60,1920x0,1" >/dev/null 2>&1
  hyprq keyword monitor "eDP-1,3840x2160@60,0x0,2" >/dev/null 2>&1
  echo "$hl"
}

trap stage_down EXIT INT TERM
```

- [ ] **Step 3: Verify `demo/env/launch-app.sh` has the X11 env**

Read `demo/env/launch-app.sh`. Confirm the `nix develop --command bash -c '...'` block exports `DISPLAY=:0 GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 STRIKE48_API_URL=https://plg.strike48.test MATRIX_TLS_INSECURE=1` and `exec ./target/debug/strikehub`. If `GDK_BACKEND=x11` or `WEBKIT_DISABLE_DMABUF_RENDERER=1` is missing, add them. If already correct, leave unchanged.

- [ ] **Step 4: Verify stage_up/down cleanly (destructive — touches live Hyprland; ensure only eDP-1 remains after)**

Run:
```bash
cd demo && bash -c '
  source env/session.sh; source env/stage.sh
  HL=$(stage_up); echo "created $HL at $(hyprq monitors -j | jq -r ".[]|select(.name==\"$HL\")|\"\(.x),\(.y)\"")"
  stage_down
  echo "after: $(hyprq monitors -j | jq -r ".[].name" | tr "\n" " ")"
'
```
Expected: prints `created HEADLESS-N at 1920,0`, then `after: eDP-1`. If any HEADLESS-* remains, re-run `stage_down` until only eDP-1 is left (never leave a stray output — it shifts the real screen).

- [ ] **Step 5: Commit**

```bash
git add demo/env/session.sh demo/env/stage.sh demo/env/launch-app.sh
git commit -m "feat(demo): env scripts — system ydotool socket, 1920-offset headless, dim off, grim helper"
```

---

### Task 8: agent.ts — Claude vision act() + verify()

**Files:**
- Create: `demo/capture/agent.ts`

**Interfaces:**
- Consumes: `@anthropic-ai/sdk`, `ANTHROPIC_API_KEY` from env.
- Produces:
  - `async act(instruction: string, pngBuffer: Buffer, dims: {width:number;height:number}): Promise<{ x: number; y: number; reasoning: string }>` — asks Claude for the screenshot-pixel click point via a forced tool call.
  - `async verify(expectation: string, pngBuffer: Buffer): Promise<{ satisfied: boolean; reasoning: string }>` — forced tool call returning a boolean.
  - Model `claude-opus-4-8`. No unit test (integration-exercised in Task 9 dry-run/probe); this task's deliverable is verified by a live smoke call in Step 3.

- [ ] **Step 1: Write `demo/capture/agent.ts`**

```ts
import Anthropic from "@anthropic-ai/sdk";

const client = new Anthropic(); // reads ANTHROPIC_API_KEY
const MODEL = "claude-opus-4-8";

function imageBlock(png: Buffer): Anthropic.ImageBlockParam {
  return { type: "image", source: { type: "base64", media_type: "image/png", data: png.toString("base64") } };
}

export async function act(
  instruction: string,
  png: Buffer,
  dims: { width: number; height: number },
): Promise<{ x: number; y: number; reasoning: string }> {
  const res = await client.messages.create({
    model: MODEL,
    max_tokens: 1024,
    tools: [{
      name: "click_at",
      description: "Report the pixel to click, in screenshot coordinates.",
      input_schema: {
        type: "object",
        properties: {
          x: { type: "integer", description: `x pixel, 0..${dims.width - 1}` },
          y: { type: "integer", description: `y pixel, 0..${dims.height - 1}` },
          reasoning: { type: "string", description: "what element you're clicking and where it is" },
        },
        required: ["x", "y", "reasoning"],
      },
    }],
    tool_choice: { type: "tool", name: "click_at" },
    messages: [{
      role: "user",
      content: [
        imageBlock(png),
        { type: "text", text: `This is a ${dims.width}x${dims.height} screenshot of the StrikeHub desktop app. Return the pixel coordinates to: ${instruction}. Coordinates are in screenshot pixels with (0,0) at the top-left.` },
      ],
    }],
  });
  const block = res.content.find((b) => b.type === "tool_use");
  if (!block || block.type !== "tool_use") throw new Error("act: no tool_use in response");
  const input = block.input as { x: number; y: number; reasoning: string };
  return { x: input.x, y: input.y, reasoning: input.reasoning };
}

export async function verify(
  expectation: string,
  png: Buffer,
): Promise<{ satisfied: boolean; reasoning: string }> {
  const res = await client.messages.create({
    model: MODEL,
    max_tokens: 1024,
    tools: [{
      name: "report_state",
      description: "Report whether the expected UI state is visible.",
      input_schema: {
        type: "object",
        properties: {
          satisfied: { type: "boolean", description: "true if the expected state is clearly visible" },
          reasoning: { type: "string", description: "what you see that supports the answer" },
        },
        required: ["satisfied", "reasoning"],
      },
    }],
    tool_choice: { type: "tool", name: "report_state" },
    messages: [{
      role: "user",
      content: [
        imageBlock(png),
        { type: "text", text: `This is a screenshot of the StrikeHub desktop app. Is the following true? "${expectation}". Answer strictly from what is visible.` },
      ],
    }],
  });
  const block = res.content.find((b) => b.type === "tool_use");
  if (!block || block.type !== "tool_use") throw new Error("verify: no tool_use in response");
  const input = block.input as { satisfied: boolean; reasoning: string };
  return { satisfied: input.satisfied, reasoning: input.reasoning };
}
```

- [ ] **Step 2: Type-check the module**

Run: `cd demo && npx tsc --noEmit` (uses the project tsconfig)
Expected: no type errors for agent.ts. (If the SDK's `ImageBlockParam`/`ToolUseBlock` names differ in the installed version, adjust to the names the compiler reports — do not guess; the compiler error names the correct type.)

- [ ] **Step 3: Live smoke test of verify() (requires ANTHROPIC_API_KEY)**

Create a throwaway PNG and confirm the API round-trips. Run:
```bash
cd demo && npx tsx -e '
import { verify } from "./capture/agent.ts";
import { execSync } from "node:child_process";
execSync("nix-shell -p imagemagick --run \"magick -size 400x200 xc:white -pointsize 40 -draw \\\"text 20,100 Hello\\\" /tmp/agent-smoke.png\"");
import { readFileSync } from "node:fs";
const png = readFileSync("/tmp/agent-smoke.png");
const r = await verify("the word Hello is visible on a white background", png);
console.log(JSON.stringify(r));
'
```
Expected: prints `{"satisfied":true,...}`. If it errors on auth, ensure `ANTHROPIC_API_KEY` is set (or `ant auth login` profile active). This proves the vision + forced-tool path works end to end.

- [ ] **Step 4: Commit**

```bash
git add demo/capture/agent.ts
git commit -m "feat(demo): Claude vision agent — act() + verify() via forced tool use"
```

---

### Task 9: runner.ts — the deterministic executor

**Files:**
- Create: `demo/capture/runner.ts` (overwrite driver.ts's role; delete `demo/capture/driver.ts`)

**Interfaces:**
- Consumes: `env/*.sh` (via `bash -c 'source ...'`), `flow.ts`, `cache.ts`, `agent.ts`, `calibrate.ts` (`screenToYdotool`, `parseHeadlessOutput`), `actions.ts`, `window.ts`.
- Produces: `recordings/<scene>.mp4` + `recordings/<scene>.timings.json` per recorded scene; honors `--refresh`, `--scene <id>`, `--from <id>`, `--no-record`.

- [ ] **Step 1: Write `demo/capture/runner.ts`**

```ts
import { execSync, spawn } from "node:child_process";
import { writeFileSync, readFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { loadFlow, type Scene, type Step } from "./flow.ts";
import { parseHeadlessOutput, screenToYdotool, type OutputInfo } from "./calibrate.ts";
import { loadCache, saveCache, getCached, setCached, type ActionCache } from "./cache.ts";
import { findStrikehub, placeCmds, activeIsStrikehub } from "./window.ts";
import { moveCmd, clickCmd, scrollCmd } from "./actions.ts";
import { act, verify } from "./agent.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const DEMO = resolve(HERE, "..");
const REC = resolve(DEMO, "recordings");
const CACHE_PATH = resolve(DEMO, "cache", "actions.json");
const FAIL_DIR = resolve(DEMO, "cache", "failures");

const args = process.argv.slice(2);
const flagRefresh = args.includes("--refresh");
const noRecord = args.includes("--no-record");
const onlyScene = args.includes("--scene") ? args[args.indexOf("--scene") + 1] : null;
const fromScene = args.includes("--from") ? args[args.indexOf("--from") + 1] : null;

const sh = (cmd: string) => execSync(cmd, { stdio: "pipe" }).toString().trim();
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const bashEnv = (fnCall: string) =>
  sh(`bash -c 'source ${DEMO}/env/session.sh; source ${DEMO}/env/stage.sh; ${fnCall}'`);

async function shootPng(headless: string): Promise<Buffer> {
  const tmp = `/tmp/runner-shot-${Date.now()}.png`;
  sh(`bash -c 'source ${DEMO}/env/session.sh; shot ${headless} ${tmp}'`);
  return readFileSync(tmp);
}

/** Poll verify() until satisfied or timeout. */
async function waitFor(expectation: string, headless: string, timeoutMs: number, stepId: string): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  let last = "";
  while (Date.now() < deadline) {
    const png = await shootPng(headless);
    const { satisfied, reasoning } = await verify(expectation, png);
    last = reasoning;
    if (satisfied) return;
    await sleep(1500);
  }
  const png = await shootPng(headless);
  mkdirSync(FAIL_DIR, { recursive: true });
  writeFileSync(resolve(FAIL_DIR, `${stepId}.png`), png);
  throw new Error(`verify timeout for "${expectation}" (last: ${last}) — screenshot saved`);
}

async function resolveClick(step: Step, out: OutputInfo, cache: ActionCache, headless: string): Promise<{ x: number; y: number }> {
  const cached = !flagRefresh ? getCached(cache, step.id) : undefined;
  let sx: number, sy: number;
  if (cached) {
    sx = cached.x; sy = cached.y;
  } else {
    const png = await shootPng(headless);
    const r = await act(step.act!, png, { width: out.width, height: out.height });
    sx = r.x; sy = r.y;
    setCached(cache, step.id, { x: sx, y: sy, instruction: step.act!, resolvedAt: new Date().toISOString() });
    saveCache(CACHE_PATH, cache);
    console.log(`  act ${step.id}: (${sx},${sy}) — ${r.reasoning}`);
  }
  // validate within the window rect (screenshot space is 0..width/0..height on the headless output)
  if (sx < 0 || sx >= out.width || sy < 0 || sy >= out.height) {
    throw new Error(`act ${step.id}: resolved (${sx},${sy}) outside screen ${out.width}x${out.height}`);
  }
  return screenToYdotool(sx, sy, out);
}

async function runStep(step: Step, out: OutputInfo, cache: ActionCache, headless: string) {
  if (step.scroll) {
    for (let i = 0; i < step.scroll.repeat; i++) {
      sh(scrollCmd(step.scroll.dy));
      await sleep(step.scroll.settleMs);
    }
    return;
  }
  if (step.act) {
    const { x, y } = await resolveClick(step, out, cache, headless);
    sh(moveCmd(x, y));
    await sleep(300);
    if (!noRecordDisablesClicks()) sh(clickCmd());
    await sleep(500);
  }
  if (step.verify) {
    await waitFor(step.verify, headless, step.timeoutMs ?? 30000, step.id);
  }
}
function noRecordDisablesClicks() { return false; } // clicks always fire; --no-record only skips recording

async function main() {
  mkdirSync(REC, { recursive: true });
  const flow = loadFlow(resolve(HERE, "flow.json"));
  const cache = loadCache(CACHE_PATH);

  const headless = bashEnv("stage_up").split("\n").pop()!.trim();
  console.log(`headless: ${headless}`);
  const pid = sh(`bash ${DEMO}/env/launch-app.sh`);
  console.log(`app pid: ${pid}`);

  try {
    // wait for window
    let win = null;
    for (let i = 0; i < 40; i++) {
      await sleep(1000);
      try { win = findStrikehub(sh(`hyprctl clients -j`)); } catch {}
      if (win) break;
    }
    if (!win) throw new Error("no Strikehub window after 40s: " + sh(`tail -6 /tmp/strikehub-demo.log`));
    for (const cmd of placeCmds(win.address, headless)) sh(cmd);
    await sleep(2000);

    const out = parseHeadlessOutput(sh(`hyprctl monitors -j`));

    // AUTH GATE
    console.log("\n=== AUTH: driving to Sign In ===");
    await runStep(flow.auth.signInStep, out, cache, headless);
    console.log("\n*** Complete the OAuth login in the browser window on your screen. Waiting... ***\n");
    await waitFor(flow.auth.readyVerify, headless, 300000, "auth.ready");
    console.log("=== authenticated ===\n");

    // SCENES
    let started = fromScene ? false : true;
    for (const scene of flow.scenes) {
      if (fromScene && scene.id === fromScene) started = true;
      if (!started) continue;
      if (onlyScene && scene.id !== onlyScene) continue;

      console.log(`▶ scene ${scene.id}: "${scene.caption}"`);
      // ensure focus (no focus-stealing during record)
      const w = findStrikehub(sh(`hyprctl clients -j`))!;
      sh(`hyprctl dispatch focuswindow address:${w.address}`);
      await sleep(400);

      const timings: Record<string, number> = { start: 0 };
      let rec: ReturnType<typeof spawn> | null = null;
      const t0 = Date.now();
      if (scene.record && !noRecord) {
        const geom = `${out.x},${out.y} ${out.width}x${out.height}`;
        rec = spawn("bash", ["-c", `WAYLAND_DISPLAY=wayland-1 wf-recorder -o ${headless} -g "${geom}" -f ${REC}/${scene.id}.mp4`], { stdio: "ignore" });
        await sleep(800);
      }
      for (const step of scene.steps) {
        await runStep(step, out, cache, headless);
        timings[`verify:${step.id}`] = (Date.now() - t0) / 1000;
      }
      timings.end = (Date.now() - t0) / 1000;
      if (rec) { rec.kill("SIGINT"); await sleep(700); }
      if (scene.record && !noRecord) {
        writeFileSync(resolve(REC, `${scene.id}.timings.json`), JSON.stringify(timings, null, 2));
      }
    }
    console.log("\ncapture complete");
  } finally {
    bashEnv("stage_down");
  }
}

main().catch((e) => { console.error(e); process.exitCode = 1; });
```

- [ ] **Step 2: Delete the obsolete driver**

Run: `cd demo && git rm capture/driver.ts capture/flow.test.ts 2>/dev/null; rm -f capture/driver.ts`
(Note: flow.test.ts was already overwritten in Task 5 — only remove driver.ts. If driver.ts referenced flow.ts's old exports, it's gone now.)
Actually: only `git rm capture/driver.ts`.

- [ ] **Step 3: Type-check**

Run: `cd demo && npx tsc --noEmit`
Expected: no errors. Fix any SDK/type mismatches the compiler names.

- [ ] **Step 4: Dry-drive without recording (destructive: launches app, manipulates headless output; needs Hyprland + ydotool group + ANTHROPIC_API_KEY; auth pauses for the human)**

Run: `cd demo && npx tsx capture/runner.ts --no-record --scene scan` in a background shell (`run_in_background: true`) since it launches a GUI and pauses for OAuth.
Expected: creates headless output, launches app, drives to Sign In, prints the "Complete the OAuth login" prompt. Operator signs in. Then it resolves `scan.click_scan` via act() (logs coords + reasoning) and polls verify() for the scan. Confirms the agent path works against the live app. Afterward, monitors return to only eDP-1 (stage_down in finally). If it fails, the failure screenshot lands in `cache/failures/`.

- [ ] **Step 5: Commit**

```bash
git add demo/capture/runner.ts
git rm demo/capture/driver.ts
git commit -m "feat(demo): deterministic runner (auth gate, cached act, live verify, per-scene record)"
```

---

### Task 10: build-scenes.sh — trim from timings + bt709 color

**Files:**
- Modify: `demo/recordings/build-scenes.sh` (consume flow.json trim + timings.json; force bt709)

**Interfaces:**
- Consumes: `recordings/<scene>.mp4`, `recordings/<scene>.timings.json`, `capture/flow.json`.
- Produces: `recordings/scenes/<scene>[_n].mp4` (CFR 30, bt709 full-range, trimmed/sped per flow.json).

- [ ] **Step 1: Rewrite `demo/recordings/build-scenes.sh`**

```bash
#!/usr/bin/env bash
# Normalize raw clips to CFR 30fps bt709 full-range, then cut per-scene trim
# segments (from capture/flow.json, anchors resolved via <scene>.timings.json).
set -euo pipefail
cd "$(dirname "$0")"
FLOW=../capture/flow.json
mkdir -p scenes norm

# Resolve a trim anchor ("start" | "end" | "end-Ns" | "verify:<stepId>" | "+Ns" relative to prev) to seconds.
# Args: <scene> <anchor> <prevSeconds>
resolve_anchor() {
  local scene="$1" a="$2" prev="$3" t
  local timings="${scene}.timings.json"
  local dur; dur=$(ffprobe -v error -show_entries format=duration -of default=nk=1:nw=1 "${scene}.mp4")
  case "$a" in
    start) echo 0 ;;
    end) echo "$dur" ;;
    end-*) echo "$(awk "BEGIN{print $dur - ${a#end-}+0}" | sed 's/s//')" ;;
    +*) echo "$(awk "BEGIN{print $prev + ${a#+}+0}" | sed 's/s//')" ;;
    verify:*) t=$(jq -r ".\"$a\" // empty" "$timings"); [ -n "$t" ] && echo "$t" || echo 0 ;;
    *) echo 0 ;;
  esac
}

# Force CFR + bt709 full-range on every cut.
cut() { # cut <src.mp4> <start> <end> <speed> <out.mp4>
  ffmpeg -y -loglevel error -ss "$2" -to "$3" -i "$1" -an \
    -vf "setpts=(1/$4)*PTS,fps=30,scale=1920:1080:out_range=full:out_color_matrix=bt709,format=yuv420p" \
    -color_range pc -colorspace bt709 -color_primaries bt709 -color_trc bt709 \
    -c:v libx264 -preset fast -crf 20 "$5"
}

for scene in $(jq -r '.scenes[]|select(.record==true)|.id' "$FLOW"); do
  [ -f "${scene}.mp4" ] || { echo "skip ${scene}: no raw clip"; continue; }
  # normalize to CFR first so setpts works
  ffmpeg -y -loglevel error -i "${scene}.mp4" -an -r 30 -vsync cfr \
    -vf "scale=1920:1080" -c:v libx264 -preset fast -crf 20 "norm/${scene}.mp4"
  n=0; prev=0
  seg_count=$(jq -r ".scenes[]|select(.id==\"$scene\")|.trim|length // 0" "$FLOW")
  if [ "$seg_count" = "0" ]; then
    cut "norm/${scene}.mp4" 0 "$(ffprobe -v error -show_entries format=duration -of default=nk=1:nw=1 norm/${scene}.mp4)" 1.0 "scenes/${scene}.mp4"
  else
    for i in $(seq 0 $((seg_count-1))); do
      fromA=$(jq -r ".scenes[]|select(.id==\"$scene\")|.trim[$i].from" "$FLOW")
      toA=$(jq -r ".scenes[]|select(.id==\"$scene\")|.trim[$i].to" "$FLOW")
      spd=$(jq -r ".scenes[]|select(.id==\"$scene\")|.trim[$i].speed" "$FLOW")
      from=$(resolve_anchor "$scene" "$fromA" "$prev")
      to=$(resolve_anchor "$scene" "$toA" "$from")
      cut "norm/${scene}.mp4" "$from" "$to" "$spd" "scenes/${scene}_${n}.mp4"
      prev="$to"; n=$((n+1))
    done
  fi
done

echo "=== scene clips ==="
for f in scenes/*.mp4; do echo "$f -> $(ffprobe -v error -show_entries format=duration -of default=nk=1:nw=1 "$f")s"; done
```

- [ ] **Step 2: Test the anchor math + bt709 on a synthetic clip (no capture needed)**

Run:
```bash
cd demo/recordings && nix-shell -p ffmpeg jq --run '
# make a 20s CFR test clip and a timings file
ffmpeg -y -loglevel error -f lavfi -i testsrc=duration=20:size=1920x1080:rate=30 -c:v libx264 scan.mp4
echo "{\"start\":0,\"verify:scan.click_scan\":3,\"end\":20}" > scan.timings.json
./build-scenes.sh
echo "=== color tags (want bt709 / pc) ==="
ffprobe -v error -select_streams v -show_entries stream=color_range,color_space -of csv=p=0 scenes/scan_0.mp4
rm -f scan.mp4 scan.timings.json
'
```
Expected: builds `scenes/scan_0.mp4` and `scenes/scan_1.mp4` from the two scan trim segments; color tags print `pc,bt709`. (testsrc is CFR already; the normalize step is a no-op but harmless.)

- [ ] **Step 3: Commit**

```bash
git add demo/recordings/build-scenes.sh
git commit -m "feat(demo): build-scenes trims from flow.json/timings + forces bt709 full-range"
```

---

### Task 11: Remotion — TransitionSeries (fades) + manifest from scene clips

**Files:**
- Modify: `demo/remotion/gen-manifest.ts` (glob `recordings/scenes/*.mp4`, map to scenes/captions)
- Modify: `demo/remotion/src/Reel.tsx` (TransitionSeries with cross-dissolve)
- Modify: `demo/remotion/src/Scene.tsx` (bt709 note; unchanged render, but ensure staticFile)

**Interfaces:**
- Consumes: `recordings/scenes/*.mp4`, `capture/flow.json` (captions/kenBurns), ffprobe.
- Produces: `remotion/manifest.json` entries `{name, clip, caption, kenBurns, durationInFrames}`; `Reel` renders with fades between scenes.

- [ ] **Step 1: Rewrite `demo/remotion/gen-manifest.ts`**

```ts
import { execSync } from "node:child_process";
import { existsSync, writeFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { loadFlow } from "../capture/flow.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const DEMO = resolve(HERE, "..");
const SCENES = resolve(DEMO, "recordings", "scenes");
const FPS = 30;

const flow = loadFlow(resolve(DEMO, "capture", "flow.json"));
const captionById = new Map(flow.scenes.map((s) => [s.id, s]));

function dur(file: string): number {
  const out = execSync(`nix-shell -p ffmpeg --run 'ffprobe -v error -show_entries format=duration -of csv=p=0 "${file}"'`).toString().trim();
  return parseFloat(out);
}

const clips = existsSync(SCENES) ? readdirSync(SCENES).filter((f) => f.endsWith(".mp4")).sort() : [];
const manifest = [];
for (const clip of clips) {
  // clip name is "<sceneId>.mp4" or "<sceneId>_<n>.mp4"
  const sceneId = clip.replace(/\.mp4$/, "").replace(/_\d+$/, "");
  const scene = captionById.get(sceneId);
  if (!scene) { console.warn(`skip ${clip}: no scene ${sceneId} in flow`); continue; }
  const secs = dur(resolve(SCENES, clip));
  manifest.push({
    name: clip.replace(/\.mp4$/, ""),
    clip: `recordings/scenes/${clip}`,
    caption: scene.caption,
    kenBurns: scene.kenBurns ?? null,
    durationInFrames: Math.max(1, Math.round(secs * FPS)),
  });
}
writeFileSync(resolve(HERE, "manifest.json"), JSON.stringify(manifest, null, 2));
console.log(`wrote manifest.json with ${manifest.length} clips`);
```

- [ ] **Step 2: Rewrite `demo/remotion/src/Reel.tsx` to use TransitionSeries**

```tsx
import { AbsoluteFill } from "remotion";
import { TransitionSeries, linearTiming } from "@remotion/transitions";
import { fade } from "@remotion/transitions/fade";
import { Scene } from "./Scene.tsx";
import { Intro } from "./Intro.tsx";
import { Outro } from "./Outro.tsx";
import manifest from "../manifest.json" with { type: "json" };

const INTRO = 60;
const OUTRO = 60;
const XFADE = 15; // frames

export const Reel: React.FC = () => (
  <AbsoluteFill style={{ backgroundColor: "#101014" }}>
    <TransitionSeries>
      <TransitionSeries.Sequence durationInFrames={INTRO}><Intro /></TransitionSeries.Sequence>
      <TransitionSeries.Transition presentation={fade()} timing={linearTiming({ durationInFrames: XFADE })} />
      {manifest.flatMap((s: any, i: number) => {
        const seq = (
          <TransitionSeries.Sequence key={s.name} durationInFrames={s.durationInFrames}>
            <Scene src={s.clip} caption={s.caption} kenBurns={s.kenBurns} />
          </TransitionSeries.Sequence>
        );
        const trans = (
          <TransitionSeries.Transition key={`t-${s.name}`} presentation={fade()} timing={linearTiming({ durationInFrames: XFADE })} />
        );
        return [seq, trans];
      })}
      <TransitionSeries.Sequence durationInFrames={OUTRO}><Outro /></TransitionSeries.Sequence>
    </TransitionSeries>
  </AbsoluteFill>
);

export const reelDuration = () =>
  // TransitionSeries overlaps each transition by XFADE; total = sum(durations) - XFADE*(#transitions)
  INTRO + OUTRO + manifest.reduce((n: number, s: any) => n + s.durationInFrames, 0) - XFADE * (manifest.length + 1);
```

- [ ] **Step 3: Confirm Scene.tsx uses staticFile (it does from prior work) — no change needed unless missing**

Read `demo/remotion/src/Scene.tsx`; confirm it wraps `src` in `staticFile(src)` for `<OffthreadVideo>`. If not, add it. (This lets `recordings/scenes/...` resolve via the `public -> ..` symlink.)

- [ ] **Step 4: Verify the composition loads with the transitions**

Run: `cd demo && npx remotion compositions remotion/src/Root.tsx --public-dir=remotion/public` (uses nix chromium if needed: prefix with `nix-shell -p chromium --run '...'` and add `--browser-executable=$(which chromium)` if it errors).
Expected: lists a `Reel` composition without error (proves the TransitionSeries code compiles + the manifest import resolves). If `manifest.json` is empty (no scenes built yet), the composition still lists — `reelDuration()` returns intro+outro minus the single bookend transition.

- [ ] **Step 5: Commit**

```bash
git add demo/remotion/gen-manifest.ts demo/remotion/src/Reel.tsx demo/remotion/src/Scene.tsx
git commit -m "feat(demo): Remotion TransitionSeries fades + manifest from scene clips"
```

---

### Task 12: orchestrate.sh + end-to-end

**Files:**
- Modify: `demo/orchestrate.sh` (runner → build-scenes → manifest → render, bt709 render)
- Modify: `demo/README.md` (document the harness)

**Interfaces:**
- Produces: `out/strikehub-reel.mp4` from `./orchestrate.sh`.

- [ ] **Step 1: Rewrite `demo/orchestrate.sh`**

```bash
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
```

- [ ] **Step 2: Update `demo/README.md`**

Replace the "## Run" section with:

```markdown
## Run (deterministic harness)
- Full pipeline (drives app, pauses for OAuth, records, composites):
  `./orchestrate.sh`
- Re-resolve all click targets with the LLM (rebuild cache): `./orchestrate.sh --refresh`
- One scene: `npx tsx capture/runner.ts --scene easymode`
- Dry drive (no recording, for tuning): `npx tsx capture/runner.ts --no-record`
- Compose only (skip capture): `nix-shell -p ffmpeg jq --run './recordings/build-scenes.sh' && npm run manifest && ./orchestrate.sh`  (or run the render block)

Requirements: Hyprland session; operator in the `ydotool` group; `ANTHROPIC_API_KEY` in env; `../target/debug/strikehub` built. Click targets are resolved by Claude vision once and cached in `cache/actions.json` (committed) — subsequent runs replay deterministically with zero LLM `act()` calls.
```

- [ ] **Step 3: End-to-end run (destructive; needs full live environment + OAuth)**

Run `cd demo && ./orchestrate.sh` in a background shell. Sign in when prompted.
Expected: captures all 5 scenes → builds `recordings/scenes/*.mp4` (bt709) → manifest → renders `out/strikehub-reel.mp4`. Verify: the reel exists, is 1920×1080 @30, has `color_space=bt709`, and plays with fades between scenes. Confirm afterward that monitors show only `eDP-1` and no strikehub/wf-recorder procs remain. If a scene's click misses, re-run `npx tsx capture/runner.ts --refresh --scene <id>` to re-resolve that step, then re-render.

- [ ] **Step 4: Commit (including the populated cache/actions.json)**

```bash
git add demo/orchestrate.sh demo/README.md demo/cache/actions.json
git commit -m "feat(demo): orchestrator end-to-end (deterministic capture -> bt709 reel); commit resolved action cache"
```

---

## Self-Review

**Spec coverage:**
- Stagehand act() cached, verify() live → Tasks 4, 8, 9. ✓
- Claude via Anthropic API, vision, forced tool → Task 8. ✓
- Coordinate `(1920+sx)/2` fix → Task 2 (`screenToYdotool`), used in runner Task 9. ✓
- ydotool system socket, scroll primitive → Tasks 3, 7. ✓
- flow.json spec (act/verify/scroll/trim), validation → Task 5. ✓
- Auth pause-for-human → Task 9 (auth gate). ✓
- Focus during record, dim off → Tasks 7 (stage), 9 (focus before record). ✓
- bt709 full-range everywhere → Tasks 10 (build), 12 (render + retag). ✓
- CFR-before-speed → Task 10. ✓
- Scene transitions (fades) → Task 11 (TransitionSeries). ✓
- Scroll the report; scan = last movement → flow.json (Task 5) doc.scroll + scan trim `end-12s`. ✓
- Per-scene trim/pacing (no static 10s holds) → flow.json trim + Ken Burns (Tasks 5, 10, 11). ✓
- Cleanup always (trap) → Task 7 stage_down trap + Task 9 finally. ✓
- Error handling: off-window coords, verify timeout+screenshot, teardown → Task 9. ✓
- Flags --refresh/--scene/--from/--no-record → Task 9. ✓
- Reuse env/calibrate/window/Remotion/build-scenes → Tasks 2,6,7,10,11. ✓
- Determinism: replay is zero-`act()`-LLM with committed cache → Tasks 4,9,12. ✓
- Tests: calibrate/actions/cache/flow/window units; agent live smoke; runner dry-drive → Tasks 2–9. ✓

**Placeholder scan:** No TBD/TODO. flow.json act targets are natural-language (resolved by the LLM, not hardcoded coords) — correct by design, not placeholders. build-scenes anchor math is fully specified. No "similar to Task N".

**Type consistency:** `OutputInfo`/`screenToYdotool`/`parseHeadlessOutput` (calibrate) used consistently in runner. `ActionCache`/`getCached`/`setCached`/`saveCache` match Task 4↔9. `Flow`/`Scene`/`Step`/`loadFlow` match Task 5↔9↔11. `Win`/`findStrikehub`/`placeCmds` match Task 6↔9. `act`/`verify` signatures match Task 8↔9. Manifest shape `{name,clip,caption,kenBurns,durationInFrames}` identical Task 11 producer ↔ Reel.tsx consumer. moveCmd/clickCmd/scrollCmd match Task 3↔9. ✓

One noted risk (from spec): the scroll primitive against WebKitGTK is the real unknown; Task 5's `doc.scroll` uses ydotool wheel and Task 9 executes it, but if the wheel event doesn't scroll the WebView, the doc scene simply holds (still valid footage) — the fallback chain is a documented future refinement, not a blocker.
