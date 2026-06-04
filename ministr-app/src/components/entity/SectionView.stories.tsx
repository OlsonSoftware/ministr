import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { SectionView } from "./SectionView";
import { withTauriMock } from "../../../.storybook/tauri-mock";
import type { SearchResult, SymbolInfo } from "../../lib/types";

/**
 * SectionView — what a cited Ask source (and every Explore section-inspect)
 * opens into inside the global EntityPanel drawer. These stories render it at
 * the real drawer width so the §1 source-identity header is scrutinizable
 * (light + dark) without the live workspace. `useEntityPanel` no-ops outside a
 * provider, so only the IPC mock is needed.
 */

const result = (over: Partial<SearchResult> = {}): SearchResult => ({
  content_id: "ministr-core/src/retrieval/hybrid.rs#hybrid-retrieval:c2",
  resolution: "section",
  score: 0.87,
  text: `pub fn retrieve(query: &Query) -> Vec<Section> {
    // hybrid: dense vectors + sparse, then a cross-encoder rerank
    let candidates = merge(dense(query), sparse(query));
    rerank(candidates, query)
}`,
  heading_path: ["ministr-core", "retrieval", "Hybrid retrieval"],
  ...over,
});

const SYMBOLS: SymbolInfo[] = [
  {
    id: "s1",
    name: "retrieve",
    kind: "function",
    file_path: "ministr-core/src/retrieval/hybrid.rs",
    visibility: "pub",
    signature: "pub fn retrieve(query: &Query) -> Vec<Section>",
    doc_comment: "Hybrid dense + sparse retrieval, then rerank.",
    module_path: "retrieval::hybrid",
  },
  {
    id: "s2",
    name: "rerank",
    kind: "function",
    file_path: "ministr-core/src/retrieval/hybrid.rs",
    visibility: "fn",
    signature: "fn rerank(c: Vec<Candidate>, q: &Query) -> Vec<Section>",
    doc_comment: null,
    module_path: "retrieval::hybrid",
  },
];

const MOCK = {
  search_symbols: () => SYMBOLS,
  recent_coherence_events: () => [],
};

const meta = {
  title: "Entity/SectionView",
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

export const Section: Story = {
  render: () => (
    <Drawer>
      <SectionView
        entity={{ kind: "section", corpusId: "ministr", result: result() }}
      />
    </Drawer>
  ),
};

export const LowRelevance: Story = {
  render: () => (
    <Drawer>
      <SectionView
        entity={{
          kind: "section",
          corpusId: "ministr",
          result: result({ score: 0.41 }),
        }}
      />
    </Drawer>
  ),
};

export const NoHeading: Story = {
  // Falls back to the file basename as the title.
  render: () => (
    <Drawer>
      <SectionView
        entity={{
          kind: "section",
          corpusId: "ministr",
          result: result({
            heading_path: [],
            content_id: "ministr-daemon/src/ask.rs#root:c0",
          }),
        }}
      />
    </Drawer>
  ),
};
