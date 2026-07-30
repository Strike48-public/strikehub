import { Audio, Sequence, staticFile, getStaticFiles } from "remotion";
import { useMemo } from "react";

/**
 * Narration component — places voiceover audio at precise scene timings.
 *
 * Usage:
 *   <Narration voiceoverMap={voMap} introFrames={60} manifest={manifest} xfade={15} />
 *
 * Where voiceoverMap is:
 *   {
 *     intro: { file: "audio/vo/intro.wav", durationInFrames: 108 },
 *     scan: { file: "audio/vo/scan.wav", durationInFrames: 213 },
 *     ...
 *   }
 */

export interface VoiceoverEntry {
  file: string; // relative to public/ in Remotion, e.g. "audio/vo/intro.wav"
  durationInFrames: number;
}

export interface VoiceoverMap {
  intro?: VoiceoverEntry;
  outro?: VoiceoverEntry;
  [sceneKey: string]: VoiceoverEntry | undefined;
}

/** A resolved VO placement on the absolute composition timeline. */
export interface VoPlacement {
  key: string;
  file: string;
  from: number; // absolute composition frame where this VO begins
  durationInFrames: number;
}

interface NarrationProps {
  voiceoverMap: VoiceoverMap;
  introFrames: number;
  manifest: Array<{ name: string; durationInFrames: number }>;
  /**
   * Cross-dissolve overlap (frames) used by the TransitionSeries in Reel.tsx.
   * Each transition overlaps two sequences by `xfade` frames, so every scene
   * after the first starts earlier than a naive cumulative sum. Pass the SAME
   * value the TransitionSeries uses so VO placement, duck ranges, and the
   * visual timeline stay aligned. Defaults to 0 (no overlap correction).
   */
  xfade?: number;
}

/**
 * Helper: map scene names from manifest to VO keys.
 * This allows flexible mapping when manifest has more granular scenes than VO script.
 *
 * Example:
 *   manifest clips "scan_0" + "scan_1" → vo key "scan"
 *   manifest clip "easymode_0" → vo key "easymode"
 */
export const sceneToVoKey = (sceneName: string): string => {
  // build-scenes emits clips named "<sceneId>_<segmentIndex>" (e.g. "scan_0",
  // "scan_1", "doc_0"). Strip a trailing "_<number>" so every segment of a
  // scene maps to that scene's single VO key. Also tolerate the older semantic
  // suffixes (_tools/_report/_toggle/_kube/_expert) just in case.
  return sceneName.replace(/_\d+$/, "").replace(/_(tools|report|toggle|kube|expert)$/, "");
};

/**
 * Build the voiceover timeline with XFADE-corrected absolute start frames.
 *
 * This is the SINGLE source of truth for where each VO clip sits on the
 * composition timeline. Reel.tsx derives its music-ducking ranges from the
 * exact same placements, so the two can never drift out of sync.
 *
 * TransitionSeries order is: Intro, T, Scene0, T, Scene1, ..., T, Outro.
 * Each Transition overlaps its neighbours by `xfade` frames, so:
 *   - Intro starts at frame 0
 *   - Scene i (0-indexed) starts at introFrames + sum(dur[0..i-1]) - (i + 1) * xfade
 *   - Outro starts at introFrames + sum(all dur) - (nScenes + 1) * xfade
 */
export const buildVoTimeline = (
  voiceoverMap: VoiceoverMap,
  introFrames: number,
  manifest: Array<{ name: string; durationInFrames: number }>,
  xfade = 0,
): VoPlacement[] => {
  const placements: VoPlacement[] = [];

  // Narration must never overlap itself (double-talk sounds terrible). Each clip
  // starts at its scene's ideal frame, but no earlier than the previous clip
  // ends + GAP — so short scenes (share) or a long intro can't collide with
  // their neighbours. Clips stay in scene order, just serialized.
  const GAP = 6; // frames of breathing room between VO clips (~0.2s)
  let nextFree = 0;
  const place = (key: string, file: string, ideal: number, durationInFrames: number) => {
    const from = Math.max(0, ideal, nextFree);
    placements.push({ key, file, from, durationInFrames });
    nextFree = from + durationInFrames + GAP;
  };

  // Intro VO — Intro sequence has no preceding transition, so it begins at 0.
  if (voiceoverMap.intro) {
    place("intro", voiceoverMap.intro.file, 0, voiceoverMap.intro.durationInFrames);
  }

  // Scene VOs — play at the start of the first scene matching each VO key,
  // using XFADE-corrected offsets so they align with the visual timeline.
  let cum = 0; // sum of scene durations before the current scene
  const usedKeys = new Set<string>();
  manifest.forEach((scene, i) => {
    const start = introFrames + cum - (i + 1) * xfade;
    const voKey = sceneToVoKey(scene.name);
    if (!usedKeys.has(voKey) && voiceoverMap[voKey]) {
      place(voKey, voiceoverMap[voKey]!.file, start, voiceoverMap[voKey]!.durationInFrames);
      usedKeys.add(voKey);
    }
    cum += scene.durationInFrames;
  });

  // Outro VO — the Outro sequence follows the final transition.
  if (voiceoverMap.outro) {
    const outroStart = introFrames + cum - (manifest.length + 1) * xfade;
    place("outro", voiceoverMap.outro.file, outroStart, voiceoverMap.outro.durationInFrames);
  }

  return placements;
};

/**
 * True if a static file is present in the Remotion public dir. Lets the
 * composition render (and list) even when audio assets are absent at build
 * time. Uses `getStaticFiles()` rather than node:fs because this component is
 * bundled for the browser, where node:fs is unavailable.
 */
export const staticFilePresent = (path: string): boolean => {
  try {
    return getStaticFiles().some((f) => f.name === path || f.src.endsWith(path));
  } catch {
    return false;
  }
};

export const Narration: React.FC<NarrationProps> = ({ voiceoverMap, introFrames, manifest, xfade = 0 }) => {
  const timeline = useMemo(
    () => buildVoTimeline(voiceoverMap, introFrames, manifest, xfade),
    [voiceoverMap, introFrames, manifest, xfade],
  );

  return (
    <>
      {timeline
        // Guard: skip any VO whose wav is missing so the composition still renders.
        .filter((entry) => staticFilePresent(entry.file))
        .map((entry) => (
          <Sequence key={entry.key} from={entry.from} durationInFrames={entry.durationInFrames}>
            <Audio src={staticFile(entry.file)} />
          </Sequence>
        ))}
    </>
  );
};
