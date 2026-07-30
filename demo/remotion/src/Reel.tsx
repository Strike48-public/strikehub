import { AbsoluteFill } from "remotion";
import { TransitionSeries, linearTiming } from "@remotion/transitions";
import { fade } from "@remotion/transitions/fade";
import { Scene } from "./Scene.tsx";
import { Intro } from "./Intro.tsx";
import { Outro } from "./Outro.tsx";
import { Narration, MusicBed, buildVoTimeline, type VoiceoverMap, type DuckRange } from "./audio";
import manifest from "../manifest.json" with { type: "json" };

const INTRO = 60;
const OUTRO = 60;
const XFADE = 15; // frames
const FPS = 30;

// Voiceover map — durations from actual wav files (en_US-ryan-high, regenerated
// by audio/build-audio.sh). If you re-run build-audio.sh with different text or
// voice, update these to match the printed durations.
const voiceoverMap: VoiceoverMap = {
  intro: { file: "audio/vo/intro.wav", durationInFrames: Math.ceil(3.668753 * FPS) },
  scan: { file: "audio/vo/scan.wav", durationInFrames: Math.ceil(6.907937 * FPS) },
  doc: { file: "audio/vo/doc.wav", durationInFrames: Math.ceil(5.816599 * FPS) },
  share: { file: "audio/vo/share.wav", durationInFrames: Math.ceil(1.219048 * FPS) },
  easymode: { file: "audio/vo/easymode.wav", durationInFrames: Math.ceil(8.939683 * FPS) },
  pickmode: { file: "audio/vo/pickmode.wav", durationInFrames: Math.ceil(7.128526 * FPS) },
  outro: { file: "audio/vo/outro.wav", durationInFrames: Math.ceil(4.702041 * FPS) },
};

export const Reel: React.FC = () => {
  // Single source of truth for VO placement: XFADE-corrected absolute start
  // frames. Both the music-ducking ranges and <Narration> derive from this
  // exact timeline (and the same XFADE), so they can never drift apart.
  const voTimeline = buildVoTimeline(voiceoverMap, INTRO, manifest, XFADE);
  const duckRanges: DuckRange[] = voTimeline.map((p) => ({
    from: p.from,
    to: p.from + p.durationInFrames,
  }));
  const totalDuration = reelDuration();

  return (
    <AbsoluteFill style={{ backgroundColor: "#101014" }}>
      {/* Background music with ducking (renders nothing if music.mp3 absent) */}
      <MusicBed
        src="audio/music.mp3"
        baseVolume={0.55}
        duckVolume={0.22}
        duckRanges={duckRanges}
        totalDurationInFrames={totalDuration}
      />
      {/* Voiceover narration — intro + per-scene + outro, XFADE-aligned.
          Skips any VO whose wav is missing so the composition still renders. */}
      <Narration voiceoverMap={voiceoverMap} introFrames={INTRO} manifest={manifest} xfade={XFADE} />

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
