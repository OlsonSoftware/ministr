import type { Meta, StoryObj } from "@storybook/react-vite";
import type { BridgeLink } from "../../lib/types";
import { BridgeMap } from "./BridgeMap";

/**
 * BridgeMap — the codebase's cross-language seams as a bespoke, navigable map
 * (aaa-explore-bridgemap). Grouped by mechanism, each seam an export↔import
 * pair with a confidence cue; filter by mechanism + language; inspect / open.
 */

function link(over: Partial<BridgeLink> & { kind: string }): BridgeLink {
  return {
    confidence: 0.92,
    export_file: "ministr-app/src-tauri/src/commands.rs",
    export_binding_key: "survey_corpus",
    export_symbol: "survey_corpus",
    export_language: "rust",
    export_line: 412,
    import_file: "ministr-app/src/lib/api.ts",
    import_binding_key: "surveyCorpus",
    import_symbol: "surveyCorpus",
    import_language: "typescript",
    import_line: 88,
    ...over,
  };
}

const LINKS: BridgeLink[] = [
  link({ kind: "tauri_command", export_symbol: "survey_corpus", import_symbol: "surveyCorpus", export_line: 412, import_line: 88 }),
  link({ kind: "tauri_command", export_symbol: "list_sessions", import_symbol: "listSessions", export_line: 980, import_line: 142, confidence: 0.97 }),
  link({ kind: "tauri_command", export_symbol: "bridge_query", import_symbol: "bridgeQuery", export_line: 1254, import_line: 203, confidence: 0.88 }),
  link({
    kind: "pyo3_function",
    export_file: "atlas-core/src/lib.rs",
    export_symbol: "embed_batch",
    export_language: "rust",
    export_line: 64,
    import_file: "atlas/embeddings.py",
    import_symbol: "embed_batch",
    import_language: "python",
    import_line: 22,
    confidence: 0.95,
  }),
  link({
    kind: "pyo3_function",
    export_file: "atlas-core/src/lib.rs",
    export_symbol: "tokenize",
    export_language: "rust",
    export_line: 120,
    import_file: "atlas/text.py",
    import_symbol: "tokenize",
    import_language: "python",
    import_line: 9,
    confidence: 0.71,
  }),
  link({
    kind: "napi_export",
    export_file: "native/src/lib.rs",
    export_symbol: "parse_ast",
    export_language: "rust",
    export_line: 33,
    import_file: "packages/parser/index.ts",
    import_symbol: "parseAst",
    import_language: "typescript",
    import_line: 5,
    confidence: 0.9,
  }),
  link({
    kind: "http_route",
    export_file: "ministr-daemon/src/daemon.rs",
    export_symbol: "GET /api/v1/corpora/{id}/files",
    export_binding_key: "list_files",
    export_language: "rust",
    export_line: 1640,
    import_file: "ministr-app/src/lib/api.ts",
    import_symbol: "listCorpusFiles",
    import_language: "typescript",
    import_line: 51,
    confidence: 0.82,
  }),
  link({
    kind: "wasm_bindgen",
    export_file: "engine/src/wasm.rs",
    export_symbol: "render_frame",
    export_language: "rust",
    export_line: 210,
    import_file: "web/engine.ts",
    import_symbol: "renderFrame",
    import_language: "typescript",
    import_line: 14,
    confidence: 0.44,
  }),
  link({
    kind: "ffi",
    export_file: "vendor/sqlite/sqlite3.c",
    export_symbol: "sqlite3_open_v2",
    export_language: "c",
    export_line: 178000,
    import_file: "ministr-core/src/storage/sqlite.rs",
    import_symbol: "open",
    import_language: "rust",
    import_line: 64,
    confidence: 0.6,
  }),
];

const noop = () => {};

const meta = {
  title: "Code/BridgeMap",
  component: BridgeMap,
  parameters: { layout: "fullscreen" },
  args: { onInspect: noop, onOpenFile: noop },
  decorators: [
    (Story) => (
      <div className="h-[820px] w-full bg-bg">
        <div className="mx-auto h-full max-w-4xl border-x border-border bg-surface">
          <Story />
        </div>
      </div>
    ),
  ],
} satisfies Meta<typeof BridgeMap>;

export default meta;
type Story = StoryObj<typeof meta>;

/** The full map — every mechanism present, varied confidence. */
export const Rich: Story = { args: { links: LINKS } };

/** A single-mechanism project (only Tauri commands). */
export const SingleMechanism: Story = {
  args: { links: LINKS.filter((l) => l.kind === "tauri_command") },
};

/** A single-language project — honest empty state. */
export const Empty: Story = { args: { links: [] } };

/** First load — mapping the seams. */
export const Loading: Story = { args: { links: [], loading: true } };
