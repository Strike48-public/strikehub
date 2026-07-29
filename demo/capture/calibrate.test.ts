import { test } from "node:test";
import assert from "node:assert/strict";
import { toDevice, parseHeadlessOutput, screenToYdotool } from "./calibrate.ts";

test("toDevice divides by scale and rounds", () => {
  assert.deepEqual(toDevice(960, 540, 2), { x: 480, y: 270 });
  assert.deepEqual(toDevice(1920, 1080, 2), { x: 960, y: 540 });
  assert.deepEqual(toDevice(101, 101, 2), { x: 51, y: 51 }); // rounds .5 up
  assert.deepEqual(toDevice(800, 600, 1), { x: 800, y: 600 });
});

test("parseHeadlessOutput picks the HEADLESS monitor", () => {
  const json = JSON.stringify([
    { name: "eDP-1", width: 3840, height: 2160, x: 0, y: 0, scale: 2 },
    { name: "HEADLESS-6", width: 1920, height: 1080, x: 5000, y: 0, scale: 1 },
  ]);
  const o = parseHeadlessOutput(json);
  assert.equal(o.name, "HEADLESS-6");
  assert.equal(o.scale, 1);
  assert.equal(o.x, 5000);
});

test("parseHeadlessOutput throws when none present", () => {
  const json = JSON.stringify([{ name: "eDP-1", width: 3840, height: 2160, x: 0, y: 0, scale: 2 }]);
  assert.throws(() => parseHeadlessOutput(json), /no HEADLESS/i);
});

test("screenToYdotool applies (offsetX + sx)/scale and sy/scale", () => {
  const out = { name: "HEADLESS-1", width: 1920, height: 1080, x: 1920, y: 0, scale: 2 };
  // screenshot center (960,540) on a 1920-offset scale-2 output -> global (2880,540) -> ydotool (1440,270)
  assert.deepEqual(screenToYdotool(960, 540, out), { x: 1440, y: 270 });
  // screenshot origin (0,0) -> global (1920,0) -> ydotool (960,0)
  assert.deepEqual(screenToYdotool(0, 0, out), { x: 960, y: 0 });
  // rounding: (1920+1)/2 = 960.5 -> 961
  assert.deepEqual(screenToYdotool(1, 0, out), { x: 961, y: 0 });
});
