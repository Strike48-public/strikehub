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
// Properly escape cmd for bash -c by using double quotes and escaping special chars
const shEnv = (cmd: string) => {
  const escaped = cmd.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\$/g, '\\$').replace(/`/g, '\\`');
  try {
    return execSync(`bash -c "source ${DEMO}/env/session.sh && ${escaped}"`, { stdio: "pipe" }).toString().trim();
  } catch (e: any) {
    console.error(`Command failed: ${cmd}`);
    console.error(`Stderr: ${e.stderr?.toString() || 'none'}`);
    console.error(`Stdout: ${e.stdout?.toString() || 'none'}`);
    throw e;
  }
};

const args = process.argv.slice(2);
const only = args.includes("--scene") ? args[args.indexOf("--scene") + 1] : null;
const dryRun = args.includes("--dry-run");
const probe = args.includes("--probe");

async function main() {
  mkdirSync(REC, { recursive: true });
  // stage_up (headless output) + launch app via bash helpers, capture output name + pid
  // Note: we source stage.sh but DON'T let the trap fire — we drive cleanup from the
  // finally block below so it runs on EVERY exit path (success, throw, or no-window).
  const hl = sh(`bash -c 'source ${DEMO}/env/session.sh; source ${DEMO}/env/stage.sh; trap "" EXIT INT TERM; stage_up'`).split("\n").pop()!.trim();
  console.log(`headless output: ${hl}`);
  try {
    const pid = sh(`bash ${DEMO}/env/launch-app.sh`);
    console.log(`strikehub pid: ${pid}`);

    // wait up to 40s for the window
    let win = null;
    for (let i = 0; i < 40; i++) {
      await new Promise((r) => setTimeout(r, 1000));
      try { win = findStrikehub(shEnv(`hyprctl clients -j`)); } catch {}
      if (win) break;
    }
    if (!win) {
      // throw (don't process.exit) so the finally block cleans up the headless output
      throw new Error("NO WINDOW after 40s — log tail:\n" + sh(`tail -8 /tmp/strikehub-demo.log`));
    }
    // place on headless + fullscreen
    for (const cmd of placeCmds(win.address, hl)) shEnv(cmd);
    await new Promise((r) => setTimeout(r, 2000));

    const out = parseHeadlessOutput(shEnv(`hyprctl monitors -j`));
    // re-read geometry now that it's fullscreen on headless
    const placed = findStrikehub(shEnv(`hyprctl clients -j`))!;
    const ctx: Ctx = {
      scale: out.scale,
      win: { x: placed.x, y: placed.y, w: placed.w, h: placed.h },
      dryRun, probe,
      run: (cmd) => {
        if (dryRun) { console.log(`  [dry-run skip] ${cmd}`); return; }
        if (probe) console.log(`  [probe] ${cmd}`);
        shEnv(cmd);
      },
    };

    const list: Scene[] = only ? scenes.filter((s) => s.name === only) : scenes;
    if (only && list.length === 0) throw new Error(`no scene named ${only}`);

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
  } finally {
    // Cleanup MUST run on every exit path (success, throw, or no-window).
    sh(`bash -c 'source ${DEMO}/env/session.sh; source ${DEMO}/env/stage.sh; stage_down'`);
  }
}

main().catch((e) => { console.error(e); process.exit(1); });
