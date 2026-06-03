import type { Meta, StoryObj } from "@storybook/react-vite";
import type {
  SearchResult,
  SymbolDefinitionDetail,
  SymbolInfo,
  SymbolRef,
} from "../../lib/types";
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

function sym(name: string, kind: string, file_path: string): SymbolInfo {
  return {
    id: `sym::${name}`,
    name,
    kind,
    file_path,
    visibility: "pub",
    signature: `${kind} ${name}`,
    doc_comment: null,
    module_path: file_path.replace(/\//g, "::"),
  };
}

const SAME_FILE: SymbolInfo[] = [
  sym("QueryService", "struct", "ministr-core/src/service/query.rs"),
  sym("rerank", "function", "ministr-core/src/service/query.rs"),
  sym("SurveyRequest", "struct", "ministr-core/src/service/query.rs"),
  sym("SurveyResponse", "struct", "ministr-core/src/service/query.rs"),
];

const MENTIONS: SearchResult[] = [
  {
    content_id: "ministr-core/src/service/query.rs#survey",
    resolution: "section",
    score: 0.91,
    text: "survey embeds the query and runs cosine retrieval…",
    heading_path: ["service", "query", "survey"],
  },
  {
    content_id: "docs/retrieval.md#how-survey-works",
    resolution: "section",
    score: 0.78,
    text: "How survey ranks and reranks results…",
    heading_path: ["Retrieval", "How survey works"],
  },
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
    onOpenSymbol: noop,
    onOpenSection: noop,
    onAsk: noop,
  },
  decorators: [
    (Story, ctx) => {
      const w = (ctx.parameters.frameWidth as number | undefined) ?? 420;
      const host = ctx.parameters.hostLabel as string | undefined;
      return (
        <div className="h-[760px] w-full bg-bg p-6 grid place-items-center">
          <div
            style={{ width: w }}
            className="flex h-full max-w-full flex-col overflow-hidden rounded-lg border border-border bg-surface"
          >
            {host && (
              <div className="flex h-11 shrink-0 items-center border-b border-border bg-surface-overlay px-4 font-mono text-xs font-semibold text-text">
                {host}
              </div>
            )}
            <div className="min-h-0 flex-1">
              <Story />
            </div>
          </div>
        </div>
      );
    },
  ],
} satisfies Meta<typeof SymbolNeighborhood>;

export default meta;
type Story = StoryObj<typeof meta>;

/** The full neighborhood — definition + most-relevant files + grouped neighbors.
 *  In the Explore peek it offers "Inspect" to escalate to the shared panel. */
export const Rich: Story = {
  args: { definition: DEF, references: REFS, onInspect: noop },
};

/** The SAME renderer, embedded in the wide EntityPanel (no own chrome), with the
 *  deep sections — Same-file symbols + semantic Mentions. One symbol renderer. */
export const Inspector: Story = {
  parameters: { frameWidth: 720, hostLabel: "SYMBOL · survey" },
  args: {
    definition: DEF,
    references: REFS,
    embedded: true,
    sameFile: SAME_FILE,
    mentions: MENTIONS,
  },
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
