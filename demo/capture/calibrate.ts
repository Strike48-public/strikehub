export type OutputInfo = {
  name: string;
  width: number;
  height: number;
  x: number;
  y: number;
  scale: number;
};

export function toDevice(logicalX: number, logicalY: number, scale: number) {
  return { x: Math.round(logicalX / scale), y: Math.round(logicalY / scale) };
}

export function parseHeadlessOutput(hyprctlMonitorsJson: string): OutputInfo {
  const monitors = JSON.parse(hyprctlMonitorsJson) as any[];
  const m = monitors.find((mon) => String(mon.name).startsWith("HEADLESS"));
  if (!m) throw new Error("no HEADLESS output found in hyprctl monitors");
  return {
    name: m.name,
    width: m.width,
    height: m.height,
    x: m.x,
    y: m.y,
    scale: m.scale,
  };
}
