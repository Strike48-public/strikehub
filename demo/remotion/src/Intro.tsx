import { AbsoluteFill, interpolate, useCurrentFrame } from "remotion";
export const Intro: React.FC = () => {
  const f = useCurrentFrame();
  const o = interpolate(f, [0, 15, 45, 60], [0, 1, 1, 0], { extrapolateRight: "clamp" });
  return (
    <AbsoluteFill style={{ backgroundColor: "#101014", justifyContent: "center", alignItems: "center", opacity: o }}>
      <div style={{ color: "#cbd7c4", font: "700 88px Inter, system-ui, sans-serif" }}>StrikeHub</div>
    </AbsoluteFill>
  );
};
