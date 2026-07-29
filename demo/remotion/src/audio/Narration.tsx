import { Audio, Sequence, staticFile, useCurrentFrame, useVideoConfig } from "remotion";
import { useMemo } from "react";

/**
 * Narration component — places voiceover audio at precise scene timings.
 *
 * Usage:
 *   <Narration voiceoverMap={voMap} introFrames={60} manifest={manifest} />
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
  [sceneKey: string]: VoiceoverEntry | undefined;
}

interface NarrationProps {
  voiceoverMap: VoiceoverMap;
  introFrames: number;
  manifest: Array<{ name: string; durationInFrames: number }>;
}

/**
 * Helper: map scene names from manifest to VO keys.
 * This allows flexible mapping when manifest has more granular scenes than VO script.
 *
 * Example:
 *   manifest scene "scan_tools" + "scan_report" → vo key "scan"
 *   manifest scene "easymode_toggle" + "easymode_kube" → vo key "easymode"
 */
const sceneToVoKey = (sceneName: string): string => {
  // Remove suffixes like _tools, _report, _toggle, _kube, _expert
  return sceneName.replace(/_(tools|report|toggle|kube|expert)$/, "");
};

export const Narration: React.FC<NarrationProps> = ({ voiceoverMap, introFrames, manifest }) => {
  // Build a timeline: { from: frame, vo: VoiceoverEntry }
  const timeline = useMemo(() => {
    const entries: Array<{ from: number; vo: VoiceoverEntry; key: string }> = [];
    let currentFrame = 0;

    // Intro VO
    if (voiceoverMap.intro) {
      entries.push({ from: currentFrame, vo: voiceoverMap.intro, key: "intro" });
    }
    currentFrame += introFrames;

    // Scene VOs — play at start of first scene matching the VO key
    const usedKeys = new Set<string>();
    for (const scene of manifest) {
      const voKey = sceneToVoKey(scene.name);
      if (!usedKeys.has(voKey) && voiceoverMap[voKey]) {
        entries.push({ from: currentFrame, vo: voiceoverMap[voKey]!, key: voKey });
        usedKeys.add(voKey);
      }
      currentFrame += scene.durationInFrames;
    }

    // Outro VO — if defined, play at the end (user must add outro frames separately)
    // This component only handles intro + scenes; outro would need manual placement or similar logic

    return entries;
  }, [voiceoverMap, introFrames, manifest]);

  return (
    <>
      {timeline.map((entry) => (
        <Sequence key={entry.key} from={entry.from} durationInFrames={entry.vo.durationInFrames}>
          <Audio src={staticFile(entry.vo.file)} />
        </Sequence>
      ))}
    </>
  );
};
