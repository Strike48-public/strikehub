import { test } from "node:test";
import assert from "node:assert/strict";
import { scenes } from "./flow.ts";

test("scenes cover the whole-app tour with unique names and captions", () => {
  assert.ok(scenes.length >= 8);
  const names = scenes.map((s) => s.name);
  assert.equal(new Set(names).size, names.length, "scene names must be unique");
  for (const s of scenes) {
    assert.ok(s.caption.length > 0, `${s.name} needs a caption`);
    assert.ok(Array.isArray(s.steps), `${s.name} needs steps[]`);
  }
  assert.deepEqual(names.slice(0, 2), ["launch", "signin"]);
  assert.ok(names.includes("easy-mode"));
  assert.ok(names.includes("outro"));
});
