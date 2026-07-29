import { type Ctx, click, type as typeText, key, wait } from "./actions.ts";

export type Step = (ctx: Ctx) => Promise<void>;
export type Scene = {
  name: string;
  caption: string;
  kenBurns?: { from: number; to: number };
  steps: Step[];
};

// Click targets are window-relative fractions (0..1). Refine with `--probe`.
export const scenes: Scene[] = [
  {
    name: "launch",
    caption: "One window for every Strike48 tool",
    steps: [async () => { await wait(2500); }],
  },
  {
    name: "signin",
    caption: "Sign in once with your Strike48 account",
    steps: [
      async (c) => { await click(c, 0.5, 0.53); }, // Sign In button
      async () => { await wait(3000); },            // browser OAuth hands back
    ],
  },
  {
    name: "connectors",
    caption: "Live status for every connector",
    kenBurns: { from: 1.0, to: 1.12 },
    steps: [
      async (c) => { await click(c, 0.02, 0.12); }, // rail item 1
      async () => { await wait(1500); },
    ],
  },
  {
    name: "launch-pick",
    caption: "Launch Pick — your pentest copilot",
    steps: [
      async (c) => { await click(c, 0.02, 0.06); }, // Pick in rail
      async () => { await wait(3500); },            // Pick UI loads
    ],
  },
  {
    name: "pick-run",
    caption: "Kick off an assessment in one click",
    steps: [
      async (c) => { await click(c, 0.5, 0.85); },  // primary action
      async () => { await wait(4000); },
    ],
  },
  {
    name: "pick-results",
    caption: "Findings and evidence, organized",
    kenBurns: { from: 1.0, to: 1.1 },
    steps: [async () => { await wait(3000); }],
  },
  {
    name: "easy-mode",
    caption: "Easy mode keeps it simple — Advanced reveals the rest",
    steps: [
      async (c) => { await click(c, 0.01, 0.97); }, // settings gear
      async () => { await wait(1200); },
      async (c) => { await click(c, 0.5, 0.4); },   // easy-mode toggle
      async () => { await wait(1800); },
    ],
  },
  {
    name: "outro",
    caption: "StrikeHub — the unified Strike48 desktop",
    steps: [async () => { await wait(2000); }],
  },
];
