import { AbsoluteFill } from "remotion";
import { TransitionSeries, linearTiming } from "@remotion/transitions";
import { fade } from "@remotion/transitions/fade";
import { Scene } from "./Scene.tsx";
import { Intro } from "./Intro.tsx";
import { Outro } from "./Outro.tsx";
import { Narration, MusicBed, type VoiceoverMap } from "./audio";
import manifest from "../manifest.json" with { type: "json" };

const INTRO = 60;
const OUTRO = 60;
const XFADE = 15; // frames
const FPS = 30;

// Voiceover map — durations from actual wav files
const voiceoverMap: VoiceoverMap = {
  intro: { file: "audio/vo/intro.wav", durationInFrames: Math.ceil(3.610703 * FPS) },
  scan: { file: "audio/vo/scan.wav", durationInFrames: Math.ceil(7.116916 * FPS) },
  doc: { file: "audio/vo/doc.wav", durationInFrames: Math.ceil(6.153288 * FPS) },
  share: { file: "audio/vo/share.wav", durationInFrames: Math.ceil(1.207438 * FPS) },
  easymode: { file: "audio/vo/easymode.wav", durationInFrames: Math.ceil(8.417234 * FPS) },
  pickmode: { file: "audio/vo/pickmode.wav", durationInFrames: Math.ceil(7.976054 * FPS) },
  outro: { file: "audio/vo/outro.wav", durationInFrames: Math.ceil(4.911020 * FPS) },
};

// Helper: build duck ranges for music bed
const buildDuckRanges = (voMap: VoiceoverMap, introFrames: number, manifest: any[]) => {
  const ranges = [];
  let frame = 0;

  // Intro
  if (voMap.intro) {
    ranges.push({ from: frame, to: frame + voMap.intro.durationInFrames });
  }
  frame += introFrames;

  // Scenes (matches Narration's sceneToVoKey logic)
  const usedKeys = new Set<string>();
  for (const scene of manifest) {
    const voKey = scene.name.replace(/_(tools|report|toggle|kube|expert)$/, "");
    if (!usedKeys.has(voKey) && voMap[voKey]) {
      ranges.push({ from: frame, to: frame + voMap[voKey]!.durationInFrames });
      usedKeys.add(voKey);
    }
    frame += scene.durationInFrames;
  }

  // Outro
  if (voMap.outro) {
    ranges.push({ from: frame, to: frame + voMap.outro.durationInFrames });
  }

  return ranges;
};

export const Reel: React.FC = () => {
  const duckRanges = buildDuckRanges(voiceoverMap, INTRO, manifest);
  const totalDuration = reelDuration();

  return (
    <AbsoluteFill style={{ backgroundColor: "#101014" }}>
      {/* Background music with ducking */}
      <MusicBed
        src="audio/music.mp3"
        baseVolume={0.15}
        duckVolume={0.05}
        duckRanges={duckRanges}
        totalDurationInFrames={totalDuration}
      />
      {/* Voiceover narration */}
      <Narration voiceoverMap={voiceoverMap} introFrames={INTRO} manifest={manifest} />

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
};

export const reelDuration = () =>
  // TransitionSeries overlaps each transition by XFADE; total = sum(durations) - XFADE*(#transitions)
  INTRO + OUTRO + manifest.reduce((n: number, s: any) => n + s.durationInFrames, 0) - XFADE * (manifest.length + 1);
