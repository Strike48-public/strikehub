import { AbsoluteFill, Series } from "remotion";
import { Scene } from "./Scene.tsx";
import { Intro } from "./Intro.tsx";
import { Outro } from "./Outro.tsx";
import manifest from "../manifest.json" with { type: "json" };

const INTRO = 60;
const OUTRO = 60;

export const Reel: React.FC = () => (
  <AbsoluteFill style={{ backgroundColor: "#101014" }}>
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

export const reelDuration = () =>
  INTRO + OUTRO + manifest.reduce((n: number, s: any) => n + s.durationInFrames, 0);
