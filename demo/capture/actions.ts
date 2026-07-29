import { toDevice } from "./calibrate.ts";

// escape a string for safe inclusion inside a single-quoted shell word
const shSingleQuote = (s: string) => `'${s.replace(/'/g, `'\\''`)}'`;

const SOCK = "/run/ydotoold/socket";
// wrap the inner ydotool invocation as a single-quoted argument to `sg ydotool -c`
const wrap = (inner: string) => `sg ydotool -c ${shSingleQuote(`YDOTOOL_SOCKET=${SOCK} ydotool ${inner}`)}`;

export type Ctx = {
  scale: number;
  win: { x: number; y: number; w: number; h: number };
  dryRun: boolean;
  probe: boolean;
  run: (cmd: string) => void;
};

export function resolveFrac(ctx: Ctx, fx: number, fy: number) {
  const lx = ctx.win.x + fx * ctx.win.w;
  const ly = ctx.win.y + fy * ctx.win.h;
  return toDevice(lx, ly, ctx.scale);
}

export const moveCmd = (x: number, y: number) => wrap(`mousemove --absolute -- ${x} ${y}`);
export const clickCmd = () => wrap(`click 0xC0`);
export const keyCmd = (combo: string) => wrap(`key ${combo}`);
export const typeCmd = (text: string) => wrap(`type -- ${JSON.stringify(text)}`);

export const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

export async function moveTo(ctx: Ctx, fx: number, fy: number) {
  const { x, y } = resolveFrac(ctx, fx, fy);
  ctx.run(moveCmd(x, y));
  await wait(150);
}

export async function click(ctx: Ctx, fx: number, fy: number) {
  await moveTo(ctx, fx, fy);
  if (ctx.dryRun || ctx.probe) return;
  ctx.run(clickCmd());
  await wait(150);
}

export async function type(ctx: Ctx, text: string) {
  if (ctx.dryRun || ctx.probe) return;
  ctx.run(typeCmd(text));
  await wait(100);
}

export async function key(ctx: Ctx, combo: string) {
  if (ctx.dryRun || ctx.probe) return;
  ctx.run(keyCmd(combo));
  await wait(100);
}
