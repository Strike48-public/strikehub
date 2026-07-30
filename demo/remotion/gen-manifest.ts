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

// Order by flow.json scene order, then by segment index within a scene —
// NOT alphabetically (which would scramble the narrative, e.g. doc before scan).
const sceneOrder = new Map(flow.scenes.map((s, i) => [s.id, i]));
const segIndex = (f: string) => {
  const m = f.replace(/\.mp4$/, "").match(/_(\d+)$/);
  return m ? parseInt(m[1], 10) : 0;
};
const clips = existsSync(SCENES)
  ? readdirSync(SCENES)
      .filter((f) => f.endsWith(".mp4"))
      .sort((a, b) => {
        const sa = sceneOrder.get(a.replace(/\.mp4$/, "").replace(/_\d+$/, "")) ?? 999;
        const sb = sceneOrder.get(b.replace(/\.mp4$/, "").replace(/_\d+$/, "")) ?? 999;
        return sa !== sb ? sa - sb : segIndex(a) - segIndex(b);
      })
  : [];
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
