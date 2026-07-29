export const SOCK = "/run/ydotoold/socket";

/** POSIX single-quote escaping: close, escaped-quote, reopen. */
export function shQuote(s: string): string {
  return `'${s.replace(/'/g, `'\\''`)}'`;
}

const wrap = (inner: string) => `YDOTOOL_SOCKET=${SOCK} ydotool ${inner}`;

export const moveCmd = (x: number, y: number) => wrap(`mousemove --absolute -- ${x} ${y}`);
export const clickCmd = () => wrap(`click 0xC0`);
export const keyCmd = (codes: string) => wrap(`key ${codes}`);
export const typeCmd = (text: string) => wrap(`type -- ${shQuote(text)}`);
/** dy negative = scroll down (ydotool wheel convention). */
export const scrollCmd = (dy: number) => wrap(`mousemove --wheel -- 0 ${dy}`);
