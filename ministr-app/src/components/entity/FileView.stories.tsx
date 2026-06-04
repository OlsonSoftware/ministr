import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { FileView } from "./FileView";
import { withTauriMock } from "../../../.storybook/tauri-mock";
import type { FileInfo, SymbolInfo } from "../../lib/types";

/**
 * FileView — the file inspector opened from Explore's FileTree, go-to-
 * definition, and bridge/diagnostic jumps. Rendered at the real ~420px drawer
 * width so the §1 source-identity header is scrutinizable (light + dark).
 * `useEntityPanel` no-ops outside a provider, so only the IPC mock is needed.
 *
 * NOTE: no `component:` on meta — render-based stories would otherwise be
 * forced to supply `args`.
 */

const PATH = "ministr-core/src/retrieval/hybrid.rs";

const SYMBOLS: SymbolInfo[] = [
  {
    id: "s1",
    name: "retrieve",
    kind: "fn",
    file_path: PATH,
    visibility: "pub",
    signature: "pub fn retrieve(query: &Query) -> Vec<Section>",
    doc_comment: "Hybrid dense + sparse retrieval, then rerank.",
    module_path: "retrieval::hybrid",
  },
  {
    id: "s2",
    name: "HybridRetriever",
    kind: "struct",
    file_path: PATH,
    visibility: "pub",
    signature: "pub struct HybridRetriever",
    doc_comment: null,
    module_path: "retrieval::hybrid",
  },
  {
    id: "s3",
    name: "QueryBackend",
    kind: "impl",
    file_path: PATH,
    visibility: "",
    signature: "impl QueryBackend for HybridRetriever",
    doc_comment: null,
    module_path: "retrieval::hybrid",
  },
];

const FILES: FileInfo[] = [
  { path: PATH, content_hash: "abc123", mtime_ns: 0, section_count: 14 },
];

const MOCK = {
  search_symbols: () => SYMBOLS,
  bridge_query: () => [],
  recent_coherence_events: () => [],
  list_corpus_files: () => FILES,
};

const meta = {
  title: "Entity/FileView",
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

export const File: Story = {
  render: () => (
    <Drawer>
      <FileView entity={{ kind: "file", corpusId: "ministr", path: PATH }} />
    </Drawer>
  ),
};

export const DeepPath: Story = {
  render: () => (
    <Drawer>
      <FileView
        entity={{
          kind: "file",
          corpusId: "ministr",
          path: "ministr-app/src/components/surfaces/ask/AskAnswer.tsx",
        }}
      />
    </Drawer>
  ),
};
