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

/**
 * ydotool's absolute pointer axis is NOT the headless output's device space.
 * Measured empirically on this rig (`hyprctl cursorpos` probe): the compositor
 * maps ydotool absolute units across the whole global-logical layout at a fixed
 * ratio of `global_logical = ydotool * 2` — e.g. ydotool(960,540) lands at
 * global-logical (1920,1080). The factor 2 is the primary monitor's scale
 * (eDP-1 @ scale 2), the axis ydotoold calibrates against; it is independent of
 * the headless output's own scale (which is 1). Dividing by out.scale (=1) sent
 * every click to ydotool≈(1920+sx), which the axis clamps to the far corner.
 */
export const YDOTOOL_AXIS_FACTOR = 2;

/**
 * Convert a screenshot pixel (sx, sy) on the given output to ydotool absolute
 * coordinates. Screenshot pixels on the scale-1 headless output equal its local
 * logical pixels; global_logical_x = out.x (output offset) + sx, then divide by
 * the ydotool axis factor.
 */
export function screenToYdotool(sx: number, sy: number, out: OutputInfo) {
  return {
    x: Math.round((out.x + sx) / YDOTOOL_AXIS_FACTOR),
    y: Math.round(sy / YDOTOOL_AXIS_FACTOR),
  };
}
