import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { CorpusView } from "./CorpusView";
import { withTauriMock } from "../../../.storybook/tauri-mock";
import type { CorpusInfo, FileInfo } from "../../lib/types";

/**
 * CorpusView — the corpus inspector (the corpus = the whole project), opened
 * from the ScopeHeader, Fleet, and hot-file/recent-change rows. Rendered at the
 * real ~420px drawer width so the §1 identity header is scrutinizable (light +
 * dark). `useEntityPanel` no-ops outside a provider; the IPC mock covers the
 * session store, the per-corpus config editor, and the file/change feeds.
 *
 * NOTE: no `component:` on meta — render-based stories would otherwise be
 * forced to supply `args`.
 */

const corpus: CorpusInfo = {
  id: "ministr",
  display_name: "ministr",
  paths: ["/Users/alrik/Code/ministr"],
  status: { state: "idle" },
  files_indexed: 1204,
  sections_count: 12840,
  embeddings_count: 12840,
  active_sessions: 0,
  symbols_count: 41902,
  last_indexed: 0,
  model: "jina-code-v2",
};

const FILES: FileInfo[] = [
  {
    path: "ministr-core/src/retrieval/hybrid.rs",
    content_hash: "a",
    mtime_ns: 0,
    section_count: 31,
  },
  {
    path: "ministr-app/src/components/surfaces/ask/AskAnswer.tsx",
    content_hash: "b",
    mtime_ns: 0,
    section_count: 22,
  },
];

const MOCK = {
  list_supported_models: () => [
    { name: "jina-code-v2", dimension: 768, code_optimized: true },
    { name: "all-MiniLM-L6-v2", dimension: 384, code_optimized: false },
  ],
  list_sessions: () => [],
  list_corpus_files: () => FILES,
  recent_coherence_events: () => [],
};

const meta = {
  title: "Entity/CorpusView",
  parameters: { layout: "fullscreen" },
  decorators: [withTauriMock(MOCK)],
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

/** The real EntityPanel content column: a ~420px drawer with px-5 py-5. */
function Drawer({ children }: { children: ReactNode }) {
  return (
    <div className="bg-surface" style={{ width: 420 }}>
      <div className="px-5 py-5">{children}</div>
    </div>
  );
}

export const Corpus: Story = {
  render: () => (
    <Drawer>
      <CorpusView entity={{ kind: "corpus", corpus }} />
    </Drawer>
  ),
};

export const MultiPathNoModel: Story = {
  render: () => (
    <Drawer>
      <CorpusView
        entity={{
          kind: "corpus",
          corpus: {
            ...corpus,
            display_name: "monorepo",
            model: "",
            paths: ["/Users/alrik/Code/app", "/Users/alrik/Code/shared"],
          },
        }}
      />
    </Drawer>
  ),
};
