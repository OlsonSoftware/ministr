import type { Meta, StoryObj } from "@storybook/react-vite";
import type {
  CorpusInfo,
  DaemonStatus,
  SessionDetail,
} from "../../lib/types";
import {
  SessionsSurface,
  SessionCard,
  SessionCardSkeleton,
} from "./SessionsSurface";
import { surfaceContainer } from "../../lib/ui-tokens";
import { withTauriMock } from "../../../.storybook/tauri-mock";

/**
 * SessionsSurface — the live board of every agent session consuming the
 * cache. The full surface renders here via the tauri-mock `list_sessions`
 * fixture (the `useSessions` store polls invoke); per-card and skeleton
 * states are rendered directly so every state is scrutinizable.
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
];

const status = (corpora: CorpusInfo[]): DaemonStatus => ({
  version: "0.2.1",
  uptime_secs: 4210,
  memory_mb: 612,
  model: "jina-code-v2",
  model_dimension: 768,
  corpora,
  total_sessions: 3,
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

/** Live board with three sessions across the pressure range. */
export const Populated: Story = {
  decorators: [withTauriMock({ list_sessions: SESSIONS })],
  render: () => (
    <Frame>
      <SessionsSurface status={status([corpusInfo])} activeCorpusId="ministr" />
    </Frame>
  ),
};

/** No agents connected — the surface's own empty state + connect command. */
export const Empty: Story = {
  decorators: [withTauriMock({ list_sessions: [] })],
  render: () => (
    <Frame>
      <SessionsSurface status={status([corpusInfo])} activeCorpusId="ministr" />
    </Frame>
  ),
};

/** First-poll loading — the skeleton card grid (mirrors the real layout). */
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

const SERIES = [12, 14, 13, 18, 22, 21, 27, 31, 30, 38, 42];

function Cell({ children }: { children: React.ReactNode }) {
  return (
    <div className="bg-bg p-6" style={{ width: 360 }}>
      {children}
    </div>
  );
}

export const CardNormal: Story = {
  render: () => (
    <Cell>
      <SessionCard
        session={session({})}
        corpus={corpusInfo}
        series={SERIES}
        fresh={false}
        onOpen={() => {}}
      />
    </Cell>
  ),
};

export const CardElevated: Story = {
  render: () => (
    <Cell>
      <SessionCard
        session={session({
          utilization: 0.75,
          pressure_level: "elevated",
          client_name: "cursor",
        })}
        corpus={corpusInfo}
        series={SERIES}
        fresh
        onOpen={() => {}}
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
        series={SERIES}
        fresh={false}
        onOpen={() => {}}
      />
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
