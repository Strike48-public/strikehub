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
  sh(`bash -c 'source ${DEMO}/env/session.sh; source ${DEMO}/env/stage.sh; trap - EXIT INT TERM; ${fnCall}'`);
const hypr = (cmd: string) => sh(`bash -c 'source ${DEMO}/env/session.sh; ${cmd}'`);

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

let tornDown = false;
/** Idempotent teardown: kill app, remove HEADLESS-*, restore eDP-1 + dim.
 *  Runs from the normal finally AND from signal handlers, so an operator Ctrl-C
 *  during the OAuth/scan waits never leaks a headless output (which would shift
 *  the real screen). Node does not unwind `finally` on an unhandled SIGINT, so
 *  the signal handlers below are required, not redundant. */
function teardown() {
  if (tornDown) return;
  tornDown = true;
  try { bashEnv("stage_down"); } catch (e) { console.error("stage_down failed:", e); }
}
for (const sig of ["SIGINT", "SIGTERM", "SIGHUP"] as const) {
  process.on(sig, () => { console.error(`\n${sig} — tearing down stage...`); teardown(); process.exit(130); });
}

async function main() {
  mkdirSync(REC, { recursive: true });
  const flow = loadFlow(resolve(HERE, "flow.json"));
  const cache = loadCache(CACHE_PATH);

  // stage_up creates the headless output; keep it INSIDE the try so any throw
  // during stage_up / launch still hits `finally { teardown() }` and never
  // leaks a HEADLESS output (which would shift the user's real screen).
  try {
    const headless = bashEnv("stage_up").split("\n").pop()!.trim();
    console.log(`headless: ${headless}`);
    const pid = sh(`bash ${DEMO}/env/launch-app.sh`);
    console.log(`app pid: ${pid}`);

    // wait for window
    let win = null;
    for (let i = 0; i < 40; i++) {
      await sleep(1000);
      try { win = findStrikehub(hypr(`hyprctl clients -j`)); } catch {}
      if (win) break;
    }
    if (!win) throw new Error("no Strikehub window after 40s: " + sh(`tail -6 /tmp/strikehub-demo.log`));
    for (const cmd of placeCmds(win.address, headless)) hypr(cmd);
    await sleep(2000);

    const out = parseHeadlessOutput(hypr(`hyprctl monitors -j`));

    // AUTH GATE
    console.log("\n=== AUTH: driving to Sign In ===");
    // Focus the window before clicking — ydotool clicks only reach the focused
    // window; without this the Sign In click can silently miss (flaky auth).
    hypr(`hyprctl dispatch focuswindow address:${win.address}`);
    await sleep(500);
    await runStep(flow.auth.signInStep, out, cache, headless);
    console.log("\n*** Complete the OAuth login in the browser window on your screen. Waiting... ***\n");

    // After OAuth, a Preflight/connector-registration overlay may block the home
    // screen. Poll for EITHER the home (readyVerify) OR the overlay
    // (postLoginVerify); if the overlay shows, dismiss it, then wait for home.
    if (flow.auth.postLoginVerify && flow.auth.dismissStep) {
      const deadline = Date.now() + 300000;
      let dismissed = false;
      while (Date.now() < deadline && !dismissed) {
        const png = await shootPng(headless);
        if ((await verify(flow.auth.readyVerify, png)).satisfied) break; // already home
        if ((await verify(flow.auth.postLoginVerify, png)).satisfied) {
          console.log("=== preflight overlay detected — dismissing ===");
          hypr(`hyprctl dispatch focuswindow address:${win.address}`);
          await sleep(400);
          await runStep(flow.auth.dismissStep, out, cache, headless);
          dismissed = true;
        }
        await sleep(1500);
      }
    }
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
      const w = findStrikehub(hypr(`hyprctl clients -j`))!;
      hypr(`hyprctl dispatch focuswindow address:${w.address}`);
      await sleep(400);

      const timings: Record<string, number> = { start: 0 };
      let rec: ReturnType<typeof spawn> | null = null;
      const t0 = Date.now();
      if (scene.record && !noRecord) {
        const geom = `${out.x},${out.y} ${out.width}x${out.height}`;
        // wf-recorder needs XDG_RUNTIME_DIR + WAYLAND_DISPLAY to find the socket;
        // the runner's own env may lack them (spawned outside the graphical session).
        const xdg = process.env.XDG_RUNTIME_DIR || `/run/user/${process.getuid?.() ?? 1000}`;
        // Capture in RGB via libx264rgb (-x bgr0), NOT wf-recorder's default YUV
        // encode. The default squeezes the range (RGB→YUV: blacks lifted ~12→20,
        // whites 255→240) while tagging it full-range, which downstream can't
        // recover → washed-out video. libx264rgb keeps pixels identical to what
        // grim/the app render (verified: dark blacks preserved). build-scenes
        // does the single clean RGB→bt709-full conversion at cut time.
        rec = spawn(
          "wf-recorder",
          ["-o", headless, "-g", geom, "-c", "libx264rgb", "-x", "bgr0", "-p", "qp=0", "-f", `${REC}/${scene.id}.mp4`],
          {
            stdio: ["ignore", "ignore", "pipe"],
            env: { ...process.env, WAYLAND_DISPLAY: "wayland-1", XDG_RUNTIME_DIR: xdg },
          },
        );
        let recErr = "";
        rec.stderr?.on("data", (d) => { recErr += d.toString(); });
        rec.on("error", (e) => console.error(`  wf-recorder spawn error: ${e.message}`));
        rec.on("exit", (code) => {
          // wf-recorder exits 0 on SIGINT; a non-zero/early exit means it never recorded.
          if (code && code !== 0) console.error(`  wf-recorder exited ${code} for ${scene.id}: ${recErr.trim()}`);
        });
        await sleep(1200);
        if (rec.exitCode !== null && rec.exitCode !== 0) {
          throw new Error(`wf-recorder failed to start for scene ${scene.id} (exit ${rec.exitCode}): ${recErr.trim()}`);
        }
      }
      for (const step of scene.steps) {
        await runStep(step, out, cache, headless);
        timings[`verify:${step.id}`] = (Date.now() - t0) / 1000;
      }
      // Dwell on the final state (the payoff/reveal) before cutting the recorder,
      // so trims anchored at the last verify have footage to show.
      if (scene.record && !noRecord && scene.holdMs) await sleep(scene.holdMs);
      timings.end = (Date.now() - t0) / 1000;
      if (rec) { rec.kill("SIGINT"); await sleep(700); }
      if (scene.record && !noRecord) {
        writeFileSync(resolve(REC, `${scene.id}.timings.json`), JSON.stringify(timings, null, 2));
      }
    }
    console.log("\ncapture complete");
  } finally {
    teardown();
  }
}

main().catch((e) => { console.error(e); process.exitCode = 1; });
