import { AbsoluteFill, OffthreadVideo, interpolate, useCurrentFrame, useVideoConfig, staticFile } from "remotion";
import { Caption } from "./Caption.tsx";

export const Scene: React.FC<{
  src: string; caption: string; kenBurns: { from: number; to: number } | null;
}> = ({ src, caption, kenBurns }) => {
  const frame = useCurrentFrame();
  const { durationInFrames } = useVideoConfig();
  const scale = kenBurns
    ? interpolate(frame, [0, durationInFrames], [kenBurns.from, kenBurns.to])
    : 1;
  return (
    <AbsoluteFill style={{ backgroundColor: "#101014" }}>
      <AbsoluteFill style={{ transform: `scale(${scale})` }}>
        <OffthreadVideo src={staticFile(src)} />
      </AbsoluteFill>
      <Caption text={caption} />
    </AbsoluteFill>
  );
};
