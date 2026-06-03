import type { Meta, StoryObj } from "@storybook/react-vite";
import type { SymbolDefinitionDetail, SymbolRef } from "../../lib/types";
import { SymbolNeighborhood } from "./SymbolNeighborhood";

/**
 * SymbolNeighborhood — the code-graph "neighborhood" peek (aaa-explore). Where
 * the old peek showed one definition, this fuses the definition + the
 * most-relevant files + the neighbor symbols (grouped by edge kind) into one
 * navigable card. Framed at the right-panel width.
 */

const DEF: SymbolDefinitionDetail = {
  id: "sym::QueryService::survey",
  name: "survey",
  kind: "function",
  visibility: "pub",
  signature:
    "pub async fn survey(&self, req: &SurveyRequest) -> Result<SurveyResponse, QueryError>",
  doc_comment:
    "Semantic search across the corpus. Embeds the query, runs HNSW cosine\nretrieval, then reranks the top-k before returning cited sections.",
  file_path: "ministr-core/src/service/query.rs",
  line_start: 412,
  line_end: 498,
  heading_path: ["service", "query", "QueryService", "survey"],
  source_context:
    "pub async fn survey(&self, req: &SurveyRequest) -> Result<SurveyResponse, QueryError> {\n    let q = self.embedder.embed(&req.query).await?;\n    let hits = self.index.search(&q, req.top_k)?;\n    self.rerank(hits, req).await\n}",
};

function ref(
  from_name: string,
  from_file: string,
  ref_kind: string,
): SymbolRef {
  return { from_name, from_file, to_name: "survey", to_file: DEF.file_path, ref_kind };
}

const REFS: SymbolRef[] = [
  ref("handle_survey", "ministr-daemon/src/daemon.rs", "calls"),
  ref("ask_corpus", "ministr-mcp/src/server/tools.rs", "calls"),
  ref("cmd_survey", "ministr-cli/src/commands/survey.rs", "calls"),
  ref("survey_corpus", "ministr-app/src-tauri/src/commands.rs", "calls"),
  ref("QueryService", "ministr-core/src/service/mod.rs", "implements"),
  ref("use query::QueryService", "ministr-api/src/query.rs", "imports"),
  ref("ministr_survey", "ministr-mcp/src/bridge.rs", "bridge"),
  ref("survey (JS)", "ministr-app/src/lib/cloudClient.ts", "bridge"),
];

const noop = () => {};

const meta = {
  title: "Code/SymbolNeighborhood",
  component: SymbolNeighborhood,
  parameters: { layout: "fullscreen" },
  args: {
    symbolName: "survey",
    onClose: noop,
    onGoToDefinition: noop,
    onJumpRef: noop,
  },
  decorators: [
    (Story) => (
      <div className="h-[680px] w-full bg-bg p-6 grid place-items-center">
        <div className="flex h-full w-[420px] flex-col overflow-hidden rounded-lg border border-border bg-surface">
          <Story />
        </div>
      </div>
    ),
  ],
} satisfies Meta<typeof SymbolNeighborhood>;

export default meta;
type Story = StoryObj<typeof meta>;

/** The full neighborhood — definition + most-relevant files + grouped neighbors. */
export const Rich: Story = {
  args: { definition: DEF, references: REFS },
};

/** A symbol nothing references yet — honest empty neighborhood. */
export const Lonely: Story = {
  args: { definition: DEF, references: [] },
};

/** First load — mapping the neighborhood. */
export const Loading: Story = {
  args: { definition: null, references: [], loading: true },
};

/** Definition resolved but only a couple of callers. */
export const Sparse: Story = {
  args: {
    definition: DEF,
    references: [
      ref("handle_survey", "ministr-daemon/src/daemon.rs", "calls"),
      ref("ministr_survey", "ministr-mcp/src/bridge.rs", "bridge"),
    ],
  },
};
