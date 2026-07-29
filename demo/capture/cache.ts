import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

export type CachedAction = { x: number; y: number; instruction: string; resolvedAt: string };
export type ActionCache = Record<string, CachedAction>;

export function loadCache(path: string): ActionCache {
  if (!existsSync(path)) return {};
  return JSON.parse(readFileSync(path, "utf8")) as ActionCache;
}

export function saveCache(path: string, cache: ActionCache): void {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, JSON.stringify(cache, null, 2) + "\n");
}

export function getCached(cache: ActionCache, stepId: string): CachedAction | undefined {
  return cache[stepId];
}

export function setCached(cache: ActionCache, stepId: string, entry: CachedAction): void {
  cache[stepId] = entry;
}
