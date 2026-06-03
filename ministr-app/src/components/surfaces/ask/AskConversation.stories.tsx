import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CorpusInfo } from "../../../lib/types";
import { AskTurn, AskPendingTurn } from "./AskTurn";
import { ConversationHistory } from "./ConversationHistory";
import type { RecentEntry } from "./internals";
import { sourceTurn, type Thread, type Turn } from "./thread";
import { withTauriMock } from "../../../../.storybook/tauri-mock";

/**
 * Ask conversation — the threaded transformation (aaa-ask-conversation).
 * The full AskSurface thread is driven by a Tauri Channel the mock can't
 * pump, so these render the thread pieces directly: a multi-turn thread,
 * an in-flight turn, an errored turn, and the history rail.
 */

const corpusInfo: CorpusInfo = {
  id: "ministr",
  display_name: "ministr",
  paths: ["/Users/alrik/Code/ministr"],
  status: { state: "idle" },
  files_indexed: 1204,
  sections_count: 12840,
  embeddings_count: 12840,
  active_sessions: 1,
  symbols_count: 41902,
  last_indexed: Date.now() - 3_600_000,
  model: "jina-code-v2",
};

const PREVIEW_FIXTURES = {
  read_section: (args: Record<string, unknown>) => ({
    section_id: String(args.sectionId ?? "section"),
    heading_path: ["ministr-core", "lib.rs", "retrieval"],
    text: "pub fn retrieve(query: &Query) -> Vec<Section> {\n    rerank(merge(dense(query), sparse(query)))\n}",
    summary: null,
    claims_available: 3,
  }),
  symbol_definition: (args: Record<string, unknown>) => ({
    id: String(args.symbolId ?? "sym"),
    name: "IngestPipeline",
    kind: "struct",
    file_path: "ministr-core/src/ingest.rs",
    visibility: "pub",
    signature: "pub struct IngestPipeline",
    doc_comment: "",
    heading_path: ["ministr-core", "ingest", "IngestPipeline"],
    source_context: "pub struct IngestPipeline { embedder: Embedder, store: Store }",
  }),
};

const entry = (q: string, a: string, over: Partial<RecentEntry> = {}): RecentEntry => ({
  query: q,
  answer: a,
  source_ids: ["ministr-core/src/lib.rs#root:c0", "ministr-mcp/src/lib.rs#root:c0"],
  cached: false,
  model: "claude-opus-4-8",
  elapsed_ms: 2900,
  ts: Date.now(),
  ...over,
});

const doneTurn = (q: string, a: string, over: Partial<RecentEntry> = {}): Turn => ({
  id: q,
  query: q,
  status: "done",
  entry: entry(q, a, over),
  unsupported: null,
});

const THREAD_TURNS: Turn[] = [
  doneTurn(
    "How does ministr retrieve and answer questions?",
    "It runs hybrid retrieval — dense vectors plus a reranker [1] — then synthesizes a cited answer with the Claude CLI [2]. The result is the *slice that answers the question*, not the file it lives in.",
  ),
  doneTurn(
    "Where is the reranker wired in?",
    "The reranker is applied after candidate merge, in `ministr-core`'s retrieval path [1]. It re-scores the merged dense+sparse set before synthesis.",
    { elapsed_ms: 1800, cached: true },
  ),
];

function Frame({ children }: { children: React.ReactNode }) {
  return (
    <div className="@container/page bg-bg p-6" style={{ maxWidth: 780 }}>
      <div className="flex flex-col gap-6">{children}</div>
    </div>
  );
}

const meta = {
  title: "Surfaces/Ask/Conversation",
  parameters: { layout: "fullscreen" },
  decorators: [withTauriMock(PREVIEW_FIXTURES)],
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

/** A multi-turn thread: two answered questions stacked, newest last. */
export const MultiTurn: Story = {
  render: () => (
    <Frame>
      {THREAD_TURNS.map((t) => (
        <AskTurn
          key={t.id}
          turn={t}
          corpusId="ministr"
          corpus={corpusInfo}
          health={null}
          pinned={false}
          onPin={() => {}}
          onUnpin={() => {}}
          onRetry={() => {}}
          onDropSource={() => {}}
          onRemoveSource={() => {}}
        />
      ))}
    </Frame>
  ),
};

/** A citation dropped INTO the thread as a kept source block, sitting between
 *  answered turns (aaa-ask-citation-dropin). */
export const WithDroppedSource: Story = {
  render: () => {
    const dropped = sourceTurn("ministr-core/src/lib.rs#root:c0", 1);
    const turns: Turn[] = [THREAD_TURNS[0], dropped, THREAD_TURNS[1]];
    return (
      <Frame>
        {turns.map((t) => (
          <AskTurn
            key={t.id}
            turn={t}
            corpusId="ministr"
            corpus={corpusInfo}
            health={null}
            pinned={false}
            onPin={() => {}}
            onUnpin={() => {}}
            onRetry={() => {}}
            onDropSource={() => {}}
            onRemoveSource={() => {}}
          />
        ))}
      </Frame>
    );
  },
};

/** A follow-up in flight below a completed turn. */
export const FollowUpInFlight: Story = {
  render: () => (
    <Frame>
      <AskTurn
        turn={THREAD_TURNS[0]}
        corpusId="ministr"
        corpus={corpusInfo}
        health={null}
        pinned={false}
        onPin={() => {}}
        onUnpin={() => {}}
        onRetry={() => {}}
        onDropSource={() => {}}
        onRemoveSource={() => {}}
      />
      <AskPendingTurn
        query="And which model does the synthesis?"
        phase="synthesizing"
      />
    </Frame>
  ),
};

/** An errored turn within the thread. */
export const TurnError: Story = {
  render: () => (
    <Frame>
      <AskTurn
        turn={{
          id: "e",
          query: "Summarize the auth flow",
          status: "error",
          error: "inference failed: `claude` not found on PATH",
        }}
        corpusId="ministr"
        corpus={corpusInfo}
        health={{ available: false, reason: "The `claude` binary was not found.", binary_path: null }}
        pinned={false}
        onPin={() => {}}
        onUnpin={() => {}}
        onRetry={() => {}}
        onDropSource={() => {}}
        onRemoveSource={() => {}}
      />
    </Frame>
  ),
};

// ── History rail ────────────────────────────────────────────────────────────

const thread = (id: string, turns: Turn[]): Thread => ({
  id,
  corpusId: "ministr",
  turns,
  createdAt: Date.now() - 10000,
  updatedAt: Date.now(),
});

const THREADS: Thread[] = [
  thread("t1", THREAD_TURNS),
  thread("t2", [doneTurn("What are the main entry points?", "…")]),
  thread("t3", [
    doneTurn("How does indexing work?", "…"),
    doneTurn("Where are embeddings stored?", "…"),
    doneTurn("What model is the default?", "…"),
  ]),
];

export const HistoryPopulated: Story = {
  render: () => (
    <div className="bg-bg p-6" style={{ width: 300, height: 420 }}>
      <ConversationHistory
        threads={THREADS}
        activeId="t1"
        onNew={() => {}}
        onResume={() => {}}
        onDelete={() => {}}
      />
    </div>
  ),
};

export const HistoryEmpty: Story = {
  render: () => (
    <div className="bg-bg p-6" style={{ width: 300, height: 240 }}>
      <ConversationHistory
        threads={[]}
        activeId={null}
        onNew={() => {}}
        onResume={() => {}}
        onDelete={() => {}}
      />
    </div>
  ),
};
