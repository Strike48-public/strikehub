# Audio Integration Guide

This directory contains a complete voiceover + background music system for the StrikeHub demo video pipeline.

## Overview

- **Voiceover**: Piper TTS (local, deterministic, offline) synthesizes narration for intro + 5 core scenes + outro
- **Music**: Ambient background bed with volume ducking during voiceover
- **Remotion components**: `<Narration>` and `<MusicBed>` ready to drop into the composition

## Files

- `script.json` — per-scene narration script (7 entries: intro, scan, doc, share, easymode, pickmode, outro)
- `build-audio.sh` — deterministic audio pipeline: script.json → Piper → normalized wav files + music bed
- `vo/*.wav` — generated voiceover files (gitignored if >1MB total; re-generate with build-audio.sh)
- `voices/en_US-lessac-medium.onnx{,.json}` — Piper voice model (60MB; gitignored, re-download if needed)
- `music.mp3` — 90s ambient background bed (currently a generated placeholder; replace with a real track)
- `../remotion/src/audio/Narration.tsx` — Remotion component for placing VO at scene timings
- `../remotion/src/audio/MusicBed.tsx` — Remotion component for background music with ducking

## Workflow

1. **Edit script**: modify `script.json` if narration changes
2. **Rebuild audio**: run `./audio/build-audio.sh` to regenerate wav files
3. **Integrate into Remotion**: see below

## Remotion Integration

The audio components are built but NOT wired into `Reel.tsx` yet (per the isolated workstream requirement). To integrate:

### Step 1: Copy audio files to Remotion public directory

Remotion's `staticFile()` reads from `remotion/public/`. Either:
- Symlink: `ln -s ../../audio remotion/public/audio`
- Or copy: `cp -r audio/vo audio/music.mp3 remotion/public/audio/`

### Step 2: Create a voiceover map

In `Reel.tsx` (or a separate `audio-config.ts`), define the VO durations (in frames at 30fps):

```typescript
import { VoiceoverMap } from "./audio";

// At 30fps, convert seconds → frames: duration_seconds * 30
const voiceoverMap: VoiceoverMap = {
  intro: { file: "audio/vo/intro.wav", durationInFrames: Math.ceil(3.61 * 30) }, // 108 frames
  scan: { file: "audio/vo/scan.wav", durationInFrames: Math.ceil(7.12 * 30) },   // 213 frames
  doc: { file: "audio/vo/doc.wav", durationInFrames: Math.ceil(6.15 * 30) },     // 184 frames
  share: { file: "audio/vo/share.wav", durationInFrames: Math.ceil(1.21 * 30) }, // 36 frames
  easymode: { file: "audio/vo/easymode.wav", durationInFrames: Math.ceil(8.42 * 30) }, // 252 frames
  pickmode: { file: "audio/vo/pickmode.wav", durationInFrames: Math.ceil(7.98 * 30) }, // 239 frames
  outro: { file: "audio/vo/outro.wav", durationInFrames: Math.ceil(4.91 * 30) }, // 147 frames
};
```

### Step 3: Add audio components to Reel.tsx

```diff
 import { AbsoluteFill, Series } from "remotion";
 import { Scene } from "./Scene.tsx";
 import { Intro } from "./Intro.tsx";
 import { Outro } from "./Outro.tsx";
+import { Narration, MusicBed } from "./audio";
 import manifest from "../manifest.json" with { type: "json" };

+const voiceoverMap = { /* ... see Step 2 ... */ };
 const INTRO = 60;
 const OUTRO = 60;

 export const Reel: React.FC = () => (
   <AbsoluteFill style={{ backgroundColor: "#101014" }}>
+    {/* Background music with ducking */}
+    <MusicBed
+      src="audio/music.mp3"
+      baseVolume={0.15}
+      duckVolume={0.05}
+      duckRanges={[
+        { from: 0, to: voiceoverMap.intro!.durationInFrames },
+        { from: INTRO, to: INTRO + voiceoverMap.scan!.durationInFrames },
+        // ... compute offsets for each scene VO (see note below)
+      ]}
+      totalDurationInFrames={reelDuration()}
+    />
+    {/* Voiceover narration */}
+    <Narration voiceoverMap={voiceoverMap} introFrames={INTRO} manifest={manifest} />
+
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
```

**Note on duck ranges**: The `Narration` component automatically places VO at scene starts. To build the `duckRanges` array for `MusicBed`, you'll need to compute frame offsets. A helper function:

```typescript
const buildDuckRanges = (voMap: VoiceoverMap, introFrames: number, manifest: any[]) => {
  const ranges = [];
  let frame = 0;

  // Intro
  if (voMap.intro) {
    ranges.push({ from: frame, to: frame + voMap.intro.durationInFrames });
  }
  frame += introFrames;

  // Scenes (simplified — matches Narration's sceneToVoKey logic)
  const usedKeys = new Set<string>();
  for (const scene of manifest) {
    const voKey = scene.name.replace(/_(tools|report|toggle|kube|expert)$/, "");
    if (!usedKeys.has(voKey) && voMap[voKey]) {
      ranges.push({ from: frame, to: frame + voMap[voKey]!.durationInFrames });
      usedKeys.add(voKey);
    }
    frame += scene.durationInFrames;
  }

  // Outro (if needed, add OUTRO frames logic here)

  return ranges;
};
```

### Step 4: Update render commands to include audio

Previously, renders may have used `-c:a copy` or no audio flags. With audio now in the composition:

1. **Preview**: `npm run preview` (in `demo/`) — audio should play in browser
2. **Render**: `npm run render` — ensure the render command does NOT disable audio:
   - Remove any `-c:a copy` or `--muted` flags
   - Remotion will encode audio from the `<Audio>` components into the output mp4

Example render command (in `demo/package.json` or orchestrate script):
```bash
npx remotion render remotion/src/Root.tsx Reel out/reel.mp4 --codec h264
```

(No audio-specific flags needed — Remotion handles it by default.)

## Music Replacement

The current `music.mp3` is a generated 90s ambient placeholder (filtered white noise). Replace it with a real track:

1. **Find a track**: royalty-free/CC0 instrumental suitable for a tech demo. Sources:
   - [Incompetech (Kevin MacLeod)](https://incompetech.com/music/royalty-free/) — CC BY 3.0
   - [Free Music Archive](https://freemusicarchive.org/) — filter by CC0/CC BY
   - [YouTube Audio Library](https://www.youtube.com/audiolibrary/music) — free with attribution
   - Or use a music generation tool (e.g. Suno, AIVA) if you have a license

2. **Replace the file**:
   ```bash
   cp /path/to/your-track.mp3 demo/audio/music.mp3
   ```

3. **Document the license**: add a `demo/audio/MUSIC_LICENSE.txt` with the track name, artist, license, and source URL.

## Timing Alignment

The voiceover script is written to match the *intent* of the 5 core scenes from `flow.json`. However, the actual video manifest has 7 scenes (scan split into scan_tools + scan_report, etc.). The `Narration` component handles this by mapping:

- `scan_tools`, `scan_report` → `scan` VO (plays at start of scan_tools)
- `easymode_toggle`, `easymode_kube` → `easymode` VO (plays at start of easymode_toggle)
- `pickmode_toggle`, `pickmode_expert` → `pickmode` VO (plays at start of pickmode_toggle)

If the VO doesn't align perfectly (e.g. VO too long for the scene), adjust the script or scene trims:
- **Tighten VO**: edit `script.json`, re-run `build-audio.sh`
- **Extend scenes**: adjust `trim` rules in `flow.json` and re-run capture/manifest pipeline

## Determinism & Reproducibility

- Piper TTS is deterministic: same text → same audio
- `build-audio.sh` is idempotent: safe to re-run
- Voice model (60MB) and large wavs are gitignored; re-download/generate with:
  ```bash
  cd demo/audio/voices
  curl -L -o en_US-lessac-medium.onnx https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx
  curl -L -o en_US-lessac-medium.onnx.json https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx.json
  cd ../..
  ./audio/build-audio.sh
  ```

## Reverting "Captions Only" Spec

This audio system **reverses** the earlier "captions only, no audio" decision documented in the capture/render spec. The rationale:
- Voiceover makes the demo more engaging and accessible
- Piper provides deterministic, offline TTS (no API dependency)
- Music bed adds professional polish

If you need to disable audio (e.g. for a silent/captioned-only variant), simply comment out the `<Narration>` and `<MusicBed>` components in `Reel.tsx`.

## License & Attribution

- **Piper TTS**: MIT License (https://github.com/rhasspy/piper)
- **Voice model (en_US-lessac-medium)**: CC0 / Public Domain (Rhasspy project, https://huggingface.co/rhasspy/piper-voices)
- **Music**: (placeholder; replace and document license)
- **Script**: Original content for StrikeHub demo (Strike48)
