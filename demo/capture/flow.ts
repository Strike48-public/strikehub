import { readFileSync } from "node:fs";

export type Step = {
  id: string;
  act?: string;
  verify?: string;
  scroll?: { dy: number; repeat: number; settleMs: number };
  timeoutMs?: number;
};
export type TrimSeg = { from: string; to: string; speed: number };
export type Scene = {
  id: string;
  caption: string;
  record: boolean;
  kenBurns?: { from: number; to: number };
  steps: Step[];
  trim?: TrimSeg[];
  /** Extra ms to keep recording after the last step, so a reveal/payoff dwells
   *  on screen instead of the recorder cutting the instant the verify passes. */
  holdMs?: number;
};
export type Flow = {
  version: number;
  output: { headlessName: string; width: number; height: number; fps: number };
  auth: {
    signInStep: Step;
    readyVerify: string;
    /** After OAuth, a Preflight/registration overlay may block the home screen.
     *  If `postLoginVerify` matches, the runner dismisses it via `dismissStep`
     *  before waiting for `readyVerify`. Both optional — absent = no overlay. */
    postLoginVerify?: string;
    dismissStep?: Step;
  };
  scenes: Scene[];
};

export function validateFlow(obj: unknown): Flow {
  const f = obj as Flow;
  if (!f || typeof f !== "object") throw new Error("flow: not an object");
  if (typeof f.version !== "number") throw new Error("flow: missing version");
  if (!f.output?.headlessName) throw new Error("flow: missing output.headlessName");
  if (!f.auth?.signInStep?.act) throw new Error("flow: missing auth.signInStep.act");
  if (!Array.isArray(f.scenes) || f.scenes.length === 0) throw new Error("flow: no scenes");
  const sceneIds = new Set<string>();
  const stepIds = new Set<string>();
  for (const sc of f.scenes) {
    if (sceneIds.has(sc.id)) throw new Error(`flow: duplicate scene id ${sc.id}`);
    sceneIds.add(sc.id);
    for (const st of sc.steps) {
      if (stepIds.has(st.id)) throw new Error(`flow: duplicate step id ${st.id}`);
      stepIds.add(st.id);
      if (!st.act && !st.verify && !st.scroll)
        throw new Error(`flow: step ${st.id} must have act, verify, or scroll`);
    }
  }
  return f;
}

export function loadFlow(path: string): Flow {
  return validateFlow(JSON.parse(readFileSync(path, "utf8")));
}
