import { test } from "node:test";
import assert from "node:assert/strict";
import { resolveFrac, moveCmd, clickCmd, typeCmd, keyCmd } from "./actions.ts";

const ctx = { scale: 2, win: { x: 0, y: 0, w: 1920, h: 1080 }, dryRun: true, probe: false, run: () => {} };

test("resolveFrac maps window fraction to device coords", () => {
  assert.deepEqual(resolveFrac(ctx, 0.5, 0.5), { x: 480, y: 270 }); // (960,540)/2
  assert.deepEqual(resolveFrac({ ...ctx, win: { x: 100, y: 100, w: 800, h: 600 } }, 0, 0), { x: 50, y: 50 });
});

test("command builders emit exact single-quoted ydotool shell strings", () => {
  assert.equal(moveCmd(480, 270), `sg ydotool -c 'YDOTOOL_SOCKET=/run/ydotoold/socket ydotool mousemove --absolute -- 480 270'`);
  assert.equal(clickCmd(), `sg ydotool -c 'YDOTOOL_SOCKET=/run/ydotoold/socket ydotool click 0xC0'`);
  assert.equal(keyCmd("28:1 28:0"), `sg ydotool -c 'YDOTOOL_SOCKET=/run/ydotoold/socket ydotool key 28:1 28:0'`);
});

test("typeCmd is shell-safe for double quotes (single-quoted outer, C-style inner token)", () => {
  // Payload with double quotes: outer single-quotes protect it; ydotool gets a C-style "..." token.
  assert.equal(
    typeCmd('he said "hi"'),
    `sg ydotool -c 'YDOTOOL_SOCKET=/run/ydotoold/socket ydotool type -- "he said \\"hi\\""'`,
  );
});

test("typeCmd is shell-safe for single quotes (POSIX '\\'' escaping)", () => {
  // Payload with a single quote: escaped as '\'' so it survives the single-quoted wrap.
  assert.equal(
    typeCmd("it's fine"),
    `sg ydotool -c 'YDOTOOL_SOCKET=/run/ydotoold/socket ydotool type -- "it'\\''s fine"'`,
  );
});
