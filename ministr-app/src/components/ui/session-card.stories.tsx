import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CorpusInfo, SessionDetail } from "../../lib/types";
import type { SessionSample } from "../../lib/sessions";
import { SessionCard, SessionCardSkeleton } from "./session-card";

/**
 * SessionCard — the ONE rich session renderer (aaa-session-renderer-dedup),
 * built from BudgetRing / BudgetBar / Sparkline / MetricTile / StatusDot.
 *
 * Two interaction modes:
 *   - `expand`  — the Activity board: the header toggles an in-place economics
 *                 dashboard; supports lineage nesting via `child`.
 *   - `inspect` — the Projects/Tend slice: the whole card opens the deep
 *                 inspector on click; no samples → trend falls back to a bar.
 *
 * (The board composition lives in `Surfaces/Sessions`; this is the atom's own
 * catalog — every per-card state, audited by the a11y gate in both themes.)
 */

const corpusInfo: CorpusInfo = {
  id: "ministr",
  display_name: "ministr",
  paths: ["/Users/alrik/Code/ministr"],
  status: { state: "idle" },
  files_indexed: 1204,
  sections_count: 12840,
  embeddings_count: 12840,
  active_sessions: 2,
  symbols_count: 41902,
  last_indexed: Date.now() - 3_600_000,
  model: "jina-code-v2",
};

const session = (over: Partial<SessionDetail>): SessionDetail => ({
  session_id: "sess_a1b2c3d4e5f6",
  corpus_id: "ministr",
  current_turn: 7,
  delivered_count: 23,
  tokens_used: 42_000,
  tokens_remaining: 158_000,
  utilization: 0.21,
  pressure_level: "normal",
  total_deliveries: 31,
  cumulative_tokens_delivered: 88_000,
  total_tokens_saved: 46_000,
  total_evictions: 2,
  total_compressions: 4,
  dedup_hits: 19,
  compression_ratio: 0.62,
  client_name: "claude-code",
  ...over,
});

// A rising token-usage sample ring so the sparkline + burn/projection render.
const SAMPLES: SessionSample[] = Array.from({ length: 12 }, (_, i) => ({
  t: Date.now() - (12 - i) * 1500,
  tokensUsed: 20_000 + i * 2_000,
  utilization: 0.15 + i * 0.02,
  turn: 3 + i,
}));

const noop = () => {};

function Cell({
  children,
  width = 360,
}: {
  children: React.ReactNode;
  width?: number;
}) {
  return (
    <div className="bg-bg p-6" style={{ width }}>
      {children}
    </div>
  );
}

const meta = {
  title: "UI/SessionCard",
  parameters: { layout: "centered" },
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

/** `expand` mode, header closed — the board's resting state. */
export const Collapsed: Story = {
  render: () => (
    <Cell>
      <SessionCard
        session={session({})}
        corpus={corpusInfo}
        samples={SAMPLES}
        fresh={false}
        expanded={false}
        onToggle={noop}
        onOpenInspector={noop}
      />
    </Cell>
  ),
};

/** The transformation: a card expanded in place to its economics dashboard. */
export const Expanded: Story = {
  render: () => (
    <Cell>
      <SessionCard
        session={session({
          utilization: 0.75,
          pressure_level: "elevated",
          current_turn: 14,
          tokens_used: 150_000,
          tokens_remaining: 50_000,
        })}
        corpus={corpusInfo}
        samples={SAMPLES}
        fresh
        expanded
        onToggle={noop}
        onOpenInspector={noop}
      />
    </Cell>
  ),
};

/** Critical pressure — the danger tone drives the ring + verdict. */
export const Critical: Story = {
  render: () => (
    <Cell>
      <SessionCard
        session={session({
          utilization: 0.96,
          pressure_level: "critical",
          tokens_used: 192_000,
          tokens_remaining: 8_000,
        })}
        corpus={corpusInfo}
        samples={SAMPLES}
        fresh={false}
        expanded={false}
        onToggle={noop}
        onOpenInspector={noop}
      />
    </Cell>
  ),
};

/** A parent card with a nested subagent (the `child` smaller-ring density),
 *  mirroring the board's lineage indenting. */
export const Lineage: Story = {
  render: () => (
    <Cell width={380}>
      <div className="flex flex-col gap-2">
        <SessionCard
          session={session({})}
          corpus={corpusInfo}
          samples={SAMPLES}
          fresh={false}
          expanded={false}
          onToggle={noop}
          onOpenInspector={noop}
        />
        <div className="ml-3 pl-3 border-l border-border-soft flex flex-col gap-2">
          <span className="pl-0.5 font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
            1 subagent
          </span>
          <SessionCard
            session={session({
              session_id: "sess_subagent01",
              client_name: "claude-code (Task)",
              utilization: 0.15,
            })}
            corpus={corpusInfo}
            samples={SAMPLES}
            fresh={false}
            expanded={false}
            onToggle={noop}
            onOpenInspector={noop}
            child
          />
        </div>
      </div>
    </Cell>
  ),
};

/** The SAME renderer in `inspect` mode (the Projects/Tend slice): no expand
 *  chevron, the whole card opens the inspector on click. No samples → the
 *  trend falls back to a budget bar. (aaa-session-renderer-dedup) */
export const Inspect: Story = {
  render: () => (
    <Cell>
      <SessionCard
        interaction="inspect"
        session={session({
          utilization: 0.62,
          pressure_level: "elevated",
          current_turn: 14,
          tokens_used: 124_000,
          tokens_remaining: 76_000,
        })}
        corpora={[corpusInfo]}
        fresh={false}
        onOpenInspector={noop}
      />
    </Cell>
  ),
};

/** Loading placeholder. */
export const Skeleton: Story = {
  render: () => (
    <Cell>
      <SessionCardSkeleton />
    </Cell>
  ),
};
