import { test } from "node:test";
import assert from "node:assert/strict";
import { findStrikehub, placeCmds } from "./window.ts";

const clients = JSON.stringify([
  { address: "0xAAA", class: "Alacritty", monitor: 0, at: [12, 42], size: [941, 1026] },
  { address: "0xBBB", class: "Strikehub", monitor: 0, at: [967, 42], size: [941, 1026] },
]);

test("findStrikehub picks the capital-S Strikehub window", () => {
  const w = findStrikehub(clients);
  assert.equal(w?.address, "0xBBB");
  assert.equal(w?.w, 941);
  assert.equal(w?.y, 42);
});

test("findStrikehub returns null when absent", () => {
  assert.equal(findStrikehub(JSON.stringify([{ address: "0x1", class: "foo", monitor: 0, at: [0,0], size: [1,1] }])), null);
});

test("placeCmds emits batch focus+move then fullscreen", () => {
  assert.deepEqual(placeCmds("0xBBB", "HEADLESS-6"), [
    'hyprctl --batch "dispatch focuswindow address:0xBBB ; dispatch movewindow mon:HEADLESS-6"',
    "hyprctl dispatch fullscreen 1",
  ]);
});
