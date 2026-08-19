import { interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion";

export const Caption: React.FC<{ text: string }> = ({ text }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const enter = spring({ frame, fps, config: { damping: 200 } });
  const y = interpolate(enter, [0, 1], [40, 0]);
  return (
    <div style={{
      position: "absolute", bottom: 80, left: 0, right: 0,
      display: "flex", justifyContent: "center", opacity: enter, transform: `translateY(${y}px)`,
    }}>
      <div style={{
        background: "rgba(20,20,24,0.82)", color: "#cbd7c4",
        font: "500 34px Inter, system-ui, sans-serif", padding: "16px 28px",
        borderRadius: 999, border: "1px solid #4c5a44",
      }}>{text}</div>
    </div>
  );
};
