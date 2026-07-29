import { execSync } from "node:child_process";
import { existsSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { loadFlow } from "../capture/flow.ts";

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

const flow = loadFlow(resolve(DEMO, "capture", "flow.json"));
const manifest = [];
for (const s of flow.scenes) {
  const clip = resolve(REC, `${s.id}.mp4`);
  if (!existsSync(clip)) { console.warn(`skip ${s.id}: no clip`); continue; }
  const secs = durationSeconds(clip);
  manifest.push({
    name: s.id,
    clip: `recordings/${s.id}.mp4`,
    caption: s.caption,
    kenBurns: s.kenBurns ?? null,
    durationInFrames: Math.max(1, Math.round(secs * FPS)),
  });
}
writeFileSync(resolve(HERE, "manifest.json"), JSON.stringify(manifest, null, 2));
console.log(`wrote manifest.json with ${manifest.length} scenes`);
