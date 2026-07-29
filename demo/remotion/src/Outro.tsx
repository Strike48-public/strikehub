import { AbsoluteFill, interpolate, useCurrentFrame } from "remotion";
export const Outro: React.FC = () => {
  const f = useCurrentFrame();
  const o = interpolate(f, [0, 15], [0, 1], { extrapolateRight: "clamp" });
  return (
    <AbsoluteFill style={{ backgroundColor: "#101014", justifyContent: "center", alignItems: "center", opacity: o }}>
      <div style={{ color: "#cbd7c4", font: "600 52px Inter, system-ui, sans-serif" }}>
        The unified Strike48 desktop
      </div>
    </AbsoluteFill>
  );
};
