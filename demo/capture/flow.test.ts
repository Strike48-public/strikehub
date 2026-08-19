import { test } from "node:test";
import assert from "node:assert/strict";
import { validateFlow, loadFlow } from "./flow.ts";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));

test("validateFlow rejects duplicate step ids", () => {
  const bad = {
    version: 1, output: { headlessName: "HEADLESS", width: 1920, height: 1080, fps: 30 },
    auth: { signInStep: { id: "auth.signin", act: "click Sign In" }, readyVerify: "home visible" },
    scenes: [{ id: "a", caption: "c", record: true, steps: [
      { id: "dup", act: "x", verify: "y" }, { id: "dup", verify: "z" },
    ] }],
  };
  assert.throws(() => validateFlow(bad), /duplicate step id/i);
});

test("validateFlow rejects a step with no act/verify/scroll", () => {
  const bad = {
    version: 1, output: { headlessName: "HEADLESS", width: 1920, height: 1080, fps: 30 },
    auth: { signInStep: { id: "auth.signin", act: "click Sign In" }, readyVerify: "home visible" },
    scenes: [{ id: "a", caption: "c", record: true, steps: [{ id: "empty" }] }],
  };
  assert.throws(() => validateFlow(bad), /must have act, verify, or scroll/i);
});

test("the shipped flow.json is valid and has the 5 real scenes", () => {
  const flow = loadFlow(join(HERE, "flow.json"));
  const ids = flow.scenes.map((s) => s.id);
  for (const want of ["scan", "doc", "share", "easymode", "pickmode"]) {
    assert.ok(ids.includes(want), `missing scene ${want}`);
  }
  assert.ok(flow.auth.signInStep.act);
});
