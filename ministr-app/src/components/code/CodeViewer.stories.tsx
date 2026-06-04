import type { Meta, StoryObj } from "@storybook/react-vite";
import type { FileContent } from "../../lib/types";
import { CodeViewer } from "./CodeViewer";

/**
 * CodeViewer — one file rendered with Shiki highlighting, line numbers, and the
 * symbol index overlaid as clickable, hoverable hot-zones. `focusLine` scrolls a
 * line into view, FLASHES it once, and leaves it SUBTLY marked as the current
 * line (aaa-explore-codeviewer-density).
 */

const RUST_CONTENT = `//! Query service — semantic search + retrieval over the corpus.
use crate::index::HnswIndex;
use crate::storage::SqliteStorage;

/// The read-side service the MCP surface and the GUI both call.
pub struct QueryService {
    index: HnswIndex,
    storage: SqliteStorage,
}

impl QueryService {
    /// Semantic search across the corpus: embed the query, run HNSW
    /// cosine retrieval, then rerank the top-k before returning sections.
    pub async fn survey(&self, req: &SurveyRequest) -> Result<SurveyResponse> {
        let vector = self.embed(&req.query).await?;
        let hits = self.index.search(&vector, req.top_k);
        self.rerank(hits, &req.query)
    }

    /// Embed a query string into the corpus' vector space.
    async fn embed(&self, query: &str) -> Result<Vec<f32>> {
        self.embedder.encode(query).await
    }

    /// Rerank candidate hits with the cross-encoder, dropping zero vectors.
    fn rerank(&self, hits: Vec<Hit>, query: &str) -> Result<SurveyResponse> {
        let ranked = self.reranker.score(hits, query);
        Ok(SurveyResponse::from(ranked))
    }
}
`;

const RUST: FileContent = {
  path: "ministr-core/src/service/query.rs",
  lang: "rust",
  content: RUST_CONTENT,
  symbol_spans: [
    {
      id: "sym-queryservice",
      name: "QueryService",
      kind: "struct",
      signature: "pub struct QueryService",
      doc_comment: "The read-side service the MCP surface and the GUI both call.",
      line_start: 6,
      line_end: 9,
    },
    {
      id: "sym-survey",
      name: "survey",
      kind: "function",
      signature:
        "pub async fn survey(&self, req: &SurveyRequest) -> Result<SurveyResponse>",
      doc_comment:
        "Semantic search across the corpus: embed the query, run HNSW cosine retrieval, then rerank the top-k before returning sections.",
      line_start: 14,
      line_end: 18,
    },
    {
      id: "sym-embed",
      name: "embed",
      kind: "function",
      signature: "async fn embed(&self, query: &str) -> Result<Vec<f32>>",
      doc_comment: "Embed a query string into the corpus' vector space.",
      line_start: 21,
      line_end: 23,
    },
    {
      id: "sym-rerank",
      name: "rerank",
      kind: "function",
      signature:
        "fn rerank(&self, hits: Vec<Hit>, query: &str) -> Result<SurveyResponse>",
      doc_comment: "Rerank candidate hits with the cross-encoder, dropping zero vectors.",
      line_start: 26,
      line_end: 29,
    },
  ],
};

const meta = {
  title: "Code/CodeViewer",
  component: CodeViewer,
  parameters: { layout: "fullscreen" },
  args: { file: RUST, scheme: "dark", occurrences: [], onSymbolClick: () => {} },
  decorators: [
    (Story) => (
      <div className="h-[640px] w-full bg-surface-sunken">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof CodeViewer>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Highlighted file, line numbers, clickable symbols (underlined). */
export const Default: Story = {};

/** Navigated to `survey` — the line scrolls into view, flashes, then stays
 *  subtly marked as the current line. */
export const FocusLine: Story = {
  args: { focusLine: 14 },
};

/** Light theme (the Shiki theme follows `scheme`). */
export const Light: Story = {
  args: { scheme: "light", focusLine: 14 },
};
