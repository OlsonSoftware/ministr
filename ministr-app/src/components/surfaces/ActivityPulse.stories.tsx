import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ActivityEvent } from "../../lib/types";
import { ActivityPulse } from "./ActivityPulse";

/**
 * ActivityPulse — the agent's tool-call rhythm as a heartbeat
 * (aaa-activity-pulse). A time-bucketed SVG histogram over the last 5 minutes;
 * bars are call counts, segmented into cache-hits (success) and misses
 * (accent). The live gestalt atop the Activity board.
 */

// A fixed "now" so the buckets are deterministic across renders + themes.
const NOW = 1_700_000_000_000;
const WINDOW = 5 * 60 * 1000;
const TOOLS = [
  "ministr_survey",
  "ministr_read",
  "ministr_symbols",
  "ministr_references",
  "ministr_definition",
  "ministr_bridge",
];

/** Deterministic pseudo-random in [0,1) from an integer seed (no Math.random
 *  → stable story snapshots). */
function rng(seed: number): number {
  const x = Math.sin(seed * 12.9898) * 43758.5453;
  return x - Math.floor(x);
}

/** Generate `n` events spread over the window with a recency bias (more recent
 *  = busier), at the given cache-hit ratio. */
function gen(n: number, hitRatio: number, seed = 1): ActivityEvent[] {
  const out: ActivityEvent[] = [];
  for (let i = 0; i < n; i++) {
    // Bias toward "now": square the uniform so events cluster recently.
    const u = rng(seed + i);
    const ageFrac = u * u;
    const age = ageFrac * WINDOW;
    out.push({
      timestamp_ms: NOW - age,
      tool: TOOLS[Math.floor(rng(seed + i + 100) * TOOLS.length)],
      corpus_id: "ministr",
      session_id: "s1",
      summary: "",
      cache_hit: rng(seed + i + 200) < hitRatio,
      duration_ms: 20,
    });
  }
  return out;
}

const meta = {
  title: "Surfaces/ActivityPulse",
  component: ActivityPulse,
  parameters: { layout: "padded" },
  args: { now: NOW, windowMs: WINDOW },
  decorators: [
    (Story) => (
      <div className="w-full max-w-[680px] bg-bg p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ActivityPulse>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A busy burst — lots of recent calls, a mix of hits and misses. */
export const Rich: Story = { args: { events: gen(90, 0.55, 7) } };

/** A high cache-hit run — mostly green (ministr earning its keep). */
export const HighCache: Story = { args: { events: gen(60, 0.9, 3) } };

/** A trickle — a handful of scattered calls. */
export const Sparse: Story = { args: { events: gen(7, 0.4, 11) } };

/** Idle — no tool activity in the window. */
export const Quiet: Story = { args: { events: [] } };
