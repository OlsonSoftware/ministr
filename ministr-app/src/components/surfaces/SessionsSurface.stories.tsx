import type { Meta, StoryObj } from "@storybook/react-vite";
import type {
  CorpusInfo,
  DaemonStatus,
  SessionDetail,
} from "../../lib/types";
import type { SessionSample } from "../../lib/sessions";
import {
  SessionsSurface,
  SessionCard,
  SessionCardSkeleton,
} from "./SessionsSurface";
import { surfaceContainer } from "../../lib/ui-tokens";
import { withTauriMock } from "../../../.storybook/tauri-mock";

/**
 * SessionsSurface — mission control. Cards expand in place; subagents nest
 * under their parent; the board auto-sorts by pressure. The full surface
 * renders via the tauri-mock `list_sessions` fixture; per-card states are
 * rendered directly.
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

const SESSIONS: SessionDetail[] = [
  session({}),
  session({
    session_id: "sess_elevated99",
    current_turn: 14,
    tokens_used: 150_000,
    tokens_remaining: 50_000,
    utilization: 0.75,
    pressure_level: "elevated",
    client_name: "cursor",
  }),
  session({
    session_id: "sess_critical42",
    corpus_id: "ministr-private",
    current_turn: 22,
    tokens_used: 190_000,
    tokens_remaining: 10_000,
    utilization: 0.95,
    pressure_level: "critical",
    client_name: "claude-code",
  }),
  // Subagent of the first (normal) session — nests under it as lineage.
  session({
    session_id: "sess_subagent01",
    parent_session_id: "sess_a1b2c3d4e5f6",
    current_turn: 4,
    tokens_used: 30_000,
    tokens_remaining: 170_000,
    utilization: 0.15,
    pressure_level: "normal",
    client_name: "claude-code (Task)",
  }),
];

const status = (corpora: CorpusInfo[]): DaemonStatus => ({
  version: "0.2.1",
  uptime_secs: 4210,
  memory_mb: 612,
  model: "jina-code-v2",
  model_dimension: 768,
  corpora,
  total_sessions: 4,
});

function Frame({ children }: { children: React.ReactNode }) {
  return (
    <div className={surfaceContainer} style={{ height: "100vh" }}>
      {children}
    </div>
  );
}

const meta = {
  title: "Surfaces/Sessions",
  parameters: { layout: "fullscreen" },
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

/** Live board: pressure-sorted (critical floats up) with a subagent nested
 *  under its parent. */
export const Populated: Story = {
  decorators: [withTauriMock({ list_sessions: SESSIONS })],
  render: () => (
    <Frame>
      <SessionsSurface status={status([corpusInfo])} activeCorpusId="ministr" />
    </Frame>
  ),
};

export const Empty: Story = {
  decorators: [withTauriMock({ list_sessions: [] })],
  render: () => (
    <Frame>
      <SessionsSurface status={status([corpusInfo])} activeCorpusId="ministr" />
    </Frame>
  ),
};

export const Loading: Story = {
  render: () => (
    <Frame>
      <div className="p-5 grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
        {Array.from({ length: 6 }).map((_, i) => (
          <SessionCardSkeleton key={i} />
        ))}
      </div>
    </Frame>
  ),
};

// ── Per-card states ────────────────────────────────────────────────────────

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

const noop = () => {};

export const CardCollapsed: Story = {
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
export const CardExpanded: Story = {
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

export const CardCritical: Story = {
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

/** A parent card with a nested subagent, mirroring the board's lineage. */
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

export const CardSkeleton: Story = {
  render: () => (
    <Cell>
      <SessionCardSkeleton />
    </Cell>
  ),
};
