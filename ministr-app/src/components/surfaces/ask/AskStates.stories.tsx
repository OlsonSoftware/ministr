import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CorpusInfo } from "../../../lib/types";
import { AskAnswer } from "./AskAnswer";
import { AskEmpty } from "./AskEmpty";
import { AskInput } from "./AskInput";
import { AskStatus } from "./AskStatus";
import { PinnedAnswers } from "./PinnedAnswers";
import type { RecentEntry } from "./internals";
import { withTauriMock } from "../../../../.storybook/tauri-mock";

/**
 * Ask surface — per-state stories.
 *
 * The full `AskSurface` (AskSurface.stories.tsx) can only render the
 * mount-time states because the streaming pipeline rides a Tauri Channel
 * the IPC mock can't pump. These stories render the individual pieces in
 * each meaningful state so every one is scrutinizable (light + dark) in
 * Storybook — the answer card, the inline-citation glass popover, the
 * streaming status strip, the three empty variants, pinned answers, and
 * the input row.
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

const ANSWER = `ministr indexes your codebase into **sections** and embeds each one [1]. At
query time it runs hybrid retrieval — dense vectors plus a cross-encoder
reranker [2] — then synthesizes a cited answer with the Claude CLI [1, 3].

- Ingestion + embedding live in \`ministr-core\` [3].
- The MCP surface is the agent entry point [2].

The result is the *slice that answers the question*, not the file it lives in.`;

const entry = (over: Partial<RecentEntry> = {}): RecentEntry => ({
  query: "How does ministr retrieve and answer questions?",
  answer: ANSWER,
  source_ids: [
    "ministr-core/src/lib.rs#root:c0",
    "ministr-mcp/src/lib.rs#root:c0",
    "sym-ministr-core/src/ingest.rs::core::ingest::IngestPipeline",
  ],
  cached: false,
  model: "claude-opus-4-8",
  elapsed_ms: 3420,
  ts: Date.now(),
  ...over,
});

// Source-preview fixtures so the citation popover + source rows render real
// content (the IPC mock keys by command name).
const PREVIEW_FIXTURES = {
  read_section: (args: Record<string, unknown>) => ({
    section_id: String(args.sectionId ?? "section"),
    heading_path: ["ministr-core", "lib.rs", "retrieval"],
    text: "pub fn retrieve(query: &Query) -> Vec<Section> {\n    // hybrid: dense + sparse, then rerank\n    rerank(merge(dense(query), sparse(query)))\n}",
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
    doc_comment: "Drives parse → chunk → embed → persist.",
    heading_path: ["ministr-core", "ingest", "IngestPipeline"],
    source_context: "pub struct IngestPipeline {\n    embedder: Embedder,\n    store: Store,\n}",
  }),
};

function Pad({
  children,
  width = 760,
}: {
  children: React.ReactNode;
  width?: number;
}) {
  return (
    <div className="@container/page bg-bg p-6" style={{ maxWidth: width }}>
      {children}
    </div>
  );
}

const meta = {
  title: "Surfaces/Ask/States",
  parameters: { layout: "fullscreen" },
  decorators: [withTauriMock(PREVIEW_FIXTURES)],
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

// ── Answer card ────────────────────────────────────────────────────────────

export const Answer: Story = {
  render: () => (
    <Pad>
      <AskAnswer
        entry={entry()}
        corpusId="ministr"
        corpus={corpusInfo}
        verifiedUnsupported={null}
        pinned={false}
        onPin={() => {}}
        onUnpin={() => {}}
        onDropSource={() => {}}
      />
    </Pad>
  ),
};

export const AnswerPinned: Story = {
  render: () => (
    <Pad>
      <AskAnswer
        entry={entry()}
        corpusId="ministr"
        corpus={corpusInfo}
        verifiedUnsupported={null}
        pinned
        onPin={() => {}}
        onUnpin={() => {}}
        onDropSource={() => {}}
      />
    </Pad>
  ),
};

export const AnswerWithUnsupportedClaims: Story = {
  render: () => (
    <Pad>
      <AskAnswer
        entry={entry()}
        corpusId="ministr"
        corpus={corpusInfo}
        verifiedUnsupported={["ministr supports 40 languages", "it runs offline"]}
        pinned={false}
        onPin={() => {}}
        onUnpin={() => {}}
        onDropSource={() => {}}
      />
    </Pad>
  ),
};

// ── Streaming status strip ─────────────────────────────────────────────────

export const StatusThinking: Story = {
  render: () => (
    <Pad>
      <AskStatus phase="retrieving" />
    </Pad>
  ),
};

export const StatusWriting: Story = {
  render: () => (
    <Pad>
      <AskStatus phase="synthesizing" />
    </Pad>
  ),
};

export const StatusCheckingSources: Story = {
  render: () => (
    <Pad>
      <AskStatus phase="verifying" />
    </Pad>
  ),
};

export const StatusFromCache: Story = {
  render: () => (
    <Pad>
      <AskStatus phase="done" cached />
    </Pad>
  ),
};

// ── Empty / pre-question states ────────────────────────────────────────────

export const EmptyReady: Story = {
  render: () => (
    <Pad>
      <AskEmpty variant="ready" onApply={() => {}} disabled={false} />
    </Pad>
  ),
};

export const EmptyNoProject: Story = {
  render: () => (
    <Pad>
      <AskEmpty variant="no-project" onAddProject={() => {}} />
    </Pad>
  ),
};

export const EmptyInferenceUnavailable: Story = {
  render: () => (
    <Pad>
      <AskEmpty
        variant="inference-unavailable"
        reason="The `claude` binary was not found on your PATH."
      />
    </Pad>
  ),
};

// ── Pinned answers sidebar ─────────────────────────────────────────────────

export const PinnedPopulated: Story = {
  render: () => (
    <Pad width={300}>
      <div style={{ height: 360 }}>
        <PinnedAnswers
          entries={[
            entry({ query: "How does retrieval work?", cached: true }),
            entry({
              query: "Where is ingestion scheduled?",
              source_ids: ["a", "b"],
              elapsed_ms: 1200,
              ts: Date.now() - 1000,
            }),
          ]}
          activeQuery="How does retrieval work?"
          onPick={() => {}}
          onUnpin={() => {}}
        />
      </div>
    </Pad>
  ),
};

export const PinnedEmpty: Story = {
  render: () => (
    <Pad width={300}>
      <div style={{ height: 200 }}>
        <PinnedAnswers
          entries={[]}
          activeQuery=""
          onPick={() => {}}
          onUnpin={() => {}}
        />
      </div>
    </Pad>
  ),
};

// ── Input row ──────────────────────────────────────────────────────────────

function InputHarness({
  loading = false,
  disabled = false,
  withRecent = false,
}: {
  loading?: boolean;
  disabled?: boolean;
  withRecent?: boolean;
}) {
  const [query, setQuery] = useState(loading ? "How does ranking work?" : "");
  return (
    <AskInput
      query={query}
      onChange={setQuery}
      onSubmit={() => {}}
      loading={loading}
      disabled={disabled}
      disabledReason="Install the Claude CLI to enable Ask…"
      recent={
        withRecent
          ? [
              entry({ query: "What are the main entry points?" }),
              entry({ query: "How does authentication work?" }),
            ]
          : []
      }
      onPickRecent={() => {}}
      onClearRecent={() => {}}
    />
  );
}

export const InputIdle: Story = {
  render: () => (
    <Pad>
      <InputHarness />
    </Pad>
  ),
};

export const InputLoading: Story = {
  render: () => (
    <Pad>
      <InputHarness loading />
    </Pad>
  ),
};

export const InputDisabled: Story = {
  render: () => (
    <Pad>
      <InputHarness disabled />
    </Pad>
  ),
};

export const InputWithRecent: Story = {
  render: () => (
    <Pad>
      <InputHarness withRecent />
    </Pad>
  ),
};
