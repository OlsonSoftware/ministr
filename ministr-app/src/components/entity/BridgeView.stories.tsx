import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { BridgeView } from "./BridgeView";
import { withTauriMock } from "../../../.storybook/tauri-mock";
import type { BridgeLink, SymbolInfo } from "../../lib/types";

/**
 * BridgeView — the cross-language bridge inspector (ministr's signature
 * feature). Rendered at the real ~420px drawer width so the §1 identity header
 * is scrutinizable (light + dark). `useEntityPanel` no-ops outside a provider,
 * so only the IPC mock is needed.
 *
 * NOTE: no `component:` on meta — render-based stories would otherwise be
 * forced to supply `args`.
 */

const link = (over: Partial<BridgeLink> = {}): BridgeLink => ({
  kind: "pyo3_function",
  confidence: 0.94,
  export_file: "ministr-core/src/embedding/native.rs",
  export_binding_key: "embed_batch",
  export_symbol: "embed_batch",
  export_language: "rust",
  export_line: 142,
  import_file: "py/ministr/_native.pyi",
  import_binding_key: "embed_batch",
  import_symbol: "embed_batch",
  import_language: "python",
  import_line: 8,
  ...over,
});

const SYMBOLS: SymbolInfo[] = [
  {
    id: "s1",
    name: "embed_batch",
    kind: "fn",
    file_path: "ministr-core/src/embedding/native.rs",
    visibility: "pub",
    signature: "pub fn embed_batch(texts: Vec<String>) -> PyResult<Vec<Vec<f32>>>",
    doc_comment: null,
    module_path: "embedding::native",
  },
];

const SRC = `#[pyfunction]
pub fn embed_batch(texts: Vec<String>) -> PyResult<Vec<Vec<f32>>> {
    EMBEDDER.with(|e| e.encode(&texts))
}`;

const MOCK = {
  read_source_excerpt: () => SRC,
  bridge_query: () => [
    link({
      export_binding_key: "tokenize",
      export_symbol: "tokenize",
      import_binding_key: "tokenize",
      import_symbol: "tokenize",
      confidence: 0.88,
    }),
  ],
  search_symbols: () => SYMBOLS,
};

const meta = {
  title: "Entity/BridgeView",
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

export const Bridge: Story = {
  render: () => (
    <Drawer>
      <BridgeView entity={{ kind: "bridge", corpusId: "ministr", link: link() }} />
    </Drawer>
  ),
};

export const LongPairing: Story = {
  // A longer export↔import pairing must wrap cleanly in the narrow drawer.
  render: () => (
    <Drawer>
      <BridgeView
        entity={{
          kind: "bridge",
          corpusId: "ministr",
          link: link({
            kind: "tauri_command",
            export_symbol: "resolve_symbol_definition",
            import_symbol: "resolveSymbolDefinition",
            export_language: "rust",
            import_language: "typescript",
            confidence: 0.99,
          }),
        }}
      />
    </Drawer>
  ),
};
