import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CorpusInfo, SessionDetail } from "../../lib/types";
import { SessionLayer } from "./SessionLayer";

/**
 * The cross-cutting live ⚡ session layer — the ⚡N chip + the agents popover.
 * Playwright clicks the chip to open the layer; verify the pressure-sorted
 * rows, the critical tint, and the empty state in light + dark.
 */

const CORPORA: CorpusInfo[] = [
  {
    id: "ministr",
    display_name: "ministr",
    paths: ["/Users/alrik/Code/ministr"],
    status: { state: "idle" },
    files_indexed: 1284,
    sections_count: 9210,
    embeddings_count: 41233,
    active_sessions: 2,
    symbols_count: 18422,
  },
  {
    id: "ministr-private",
    display_name: "ministr-private",
    paths: ["/Users/alrik/Code/ministr-private"],
    status: { state: "idle" },
    files_indexed: 312,
    sections_count: 2104,
    embeddings_count: 9920,
    active_sessions: 1,
    symbols_count: 4210,
  },
];

function mkSession(
  over: Partial<SessionDetail> & { session_id: string },
): SessionDetail {
  return {
    corpus_id: "ministr",
    current_turn: 1,
    delivered_count: 0,
    tokens_used: 0,
    tokens_remaining: 100000,
    utilization: 0.1,
    pressure_level: "ok",
    total_deliveries: 0,
    cumulative_tokens_delivered: 0,
    total_tokens_saved: 0,
    total_evictions: 0,
    total_compressions: 0,
    dedup_hits: 0,
    compression_ratio: 0,
    ...over,
  };
}

const SESSIONS: SessionDetail[] = [
  mkSession({
    session_id: "claude-abc123def456",
    client_name: "claude-code",
    corpus_id: "ministr",
    current_turn: 14,
    utilization: 0.82,
    pressure_level: "critical",
  }),
  mkSession({
    session_id: "cursor-def456ghi789",
    client_name: "cursor",
    corpus_id: "ministr-private",
    current_turn: 6,
    utilization: 0.41,
    pressure_level: "warning",
  }),
  mkSession({
    session_id: "sub-789xyz012abc",
    client_name: "claude-code",
    corpus_id: "ministr",
    current_turn: 3,
    utilization: 0.18,
    pressure_level: "ok",
    parent_session_id: "claude-abc123def456",
  }),
];

const meta: Meta<typeof SessionLayer> = {
  title: "Workspace/SessionLayer",
  component: SessionLayer,
  decorators: [
    (Story) => (
      <div className="flex h-[440px] items-start justify-end">
        <Story />
      </div>
    ),
  ],
};
export default meta;

type Story = StoryObj<typeof SessionLayer>;

export const Populated: Story = {
  args: { sessions: SESSIONS, corpora: CORPORA },
};

export const Empty: Story = {
  args: { sessions: [], corpora: CORPORA },
};
