import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadCache, saveCache, getCached, setCached } from "./cache.ts";

test("loadCache returns empty object when file is absent", () => {
  const dir = mkdtempSync(join(tmpdir(), "cachetest-"));
  assert.deepEqual(loadCache(join(dir, "nope.json")), {});
  rmSync(dir, { recursive: true });
});

test("save then load roundtrips, and get/set work", () => {
  const dir = mkdtempSync(join(tmpdir(), "cachetest-"));
  const p = join(dir, "sub", "actions.json"); // parent dir does not exist yet
  const cache = {};
  setCached(cache, "scan.click", { x: 1440, y: 76, instruction: "click Scan", resolvedAt: "2026-07-29T00:00:00Z" });
  saveCache(p, cache);
  const loaded = loadCache(p);
  assert.equal(getCached(loaded, "scan.click")?.x, 1440);
  assert.equal(getCached(loaded, "missing"), undefined);
  rmSync(dir, { recursive: true });
});
