import { test } from "node:test";
import assert from "node:assert/strict";
import { findStrikehub, placeCmds, focusCmd, activeIsStrikehub } from "./window.ts";

const clients = JSON.stringify([
  { address: "0xAAA", class: "Alacritty", monitor: 0, at: [12, 42], size: [900, 1000] },
  { address: "0xBBB", class: "Strikehub", monitor: 1, at: [1932, 42], size: [1896, 1026] },
]);

test("findStrikehub picks the capital-S window and maps at/size", () => {
  const w = findStrikehub(clients);
  assert.equal(w?.address, "0xBBB");
  assert.equal(w?.x, 1932);
  assert.equal(w?.w, 1896);
  assert.equal(w?.monitor, 1);
});

test("findStrikehub returns null when absent", () => {
  assert.equal(findStrikehub(JSON.stringify([{ address: "0x1", class: "x", monitor: 0, at: [0,0], size: [1,1] }])), null);
});

test("placeCmds and focusCmd emit exact strings", () => {
  assert.deepEqual(placeCmds("0xBBB", "HEADLESS-6"), [
    'hyprctl --batch "dispatch focuswindow address:0xBBB ; dispatch movewindow mon:HEADLESS-6"',
    "hyprctl dispatch fullscreen 1",
  ]);
  assert.equal(focusCmd("0xBBB"), "hyprctl dispatch focuswindow address:0xBBB");
});

test("activeIsStrikehub checks the active window class", () => {
  assert.equal(activeIsStrikehub(JSON.stringify({ class: "Strikehub" })), true);
  assert.equal(activeIsStrikehub(JSON.stringify({ class: "Alacritty" })), false);
});
