import type { Meta, StoryObj } from "@storybook/react-vite";
import type { BridgeLink } from "../../lib/types";
import { BridgeFlow } from "./BridgeFlow";

/**
 * BridgeFlow — the cross-language seams as a Sankey-style flow map: export
 * languages on the left, import languages on the right, each language pair a
 * band whose thickness ∝ the number of seams. The "Flow" view of the Bridges
 * lens (the List is the detail view).
 */

function link(over: Partial<BridgeLink> & { kind: string }): BridgeLink {
  return {
    confidence: 0.9,
    export_file: "ministr-app/src-tauri/src/commands.rs",
    export_binding_key: "cmd",
    export_symbol: "cmd",
    export_language: "rust",
    export_line: 1,
    import_file: "ministr-app/src/lib/api.ts",
    import_binding_key: "cmd",
    import_symbol: "cmd",
    import_language: "typescript",
    import_line: 1,
    ...over,
  };
}

/** N seams between an export language and an import language. */
function flow(
  n: number,
  kind: string,
  exportLanguage: string,
  importLanguage: string,
): BridgeLink[] {
  return Array.from({ length: n }, (_, i) =>
    link({
      kind,
      export_language: exportLanguage,
      import_language: importLanguage,
      export_symbol: `${exportLanguage}_${i}`,
      import_symbol: `${importLanguage}_${i}`,
      export_line: 10 + i,
      import_line: 20 + i,
    }),
  );
}

const RICH: BridgeLink[] = [
  ...flow(9, "tauri_command", "rust", "typescript"),
  ...flow(4, "pyo3_function", "rust", "python"),
  ...flow(3, "napi_export", "rust", "javascript"),
  ...flow(2, "ffi", "c", "rust"),
  ...flow(2, "http_route", "rust", "typescript"),
  ...flow(1, "wasm_bindgen", "rust", "typescript"),
];

const meta = {
  title: "Code/BridgeFlow",
  component: BridgeFlow,
  args: {
    links: RICH,
    activeLang: null,
    onFilterLang: () => {},
    onDrillPair: () => {},
  },
  decorators: [
    (Story) => (
      <div className="flex h-[560px] w-[880px] flex-col overflow-hidden rounded-lg border border-border bg-surface">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof BridgeFlow>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A rich, multi-language project: Rust exports flowing to TS / Python / JS,
 *  and C → Rust via FFI — band thickness shows where the heavy seams are. */
export const Rich: Story = {};

/** Rust filtered to the active language — its node + flows stay lit, the rest
 *  dim (mirrors the shared lens language filter). */
export const FilteredToRust: Story = {
  args: { activeLang: "rust" },
};

/** A single language pair (one band) — the minimal cross-language project. */
export const SinglePair: Story = {
  args: { links: flow(5, "tauri_command", "rust", "typescript") },
};

/** Nothing matches the filter — a quiet hint (the lens handles the truly-empty
 *  project upstream). */
export const FilteredEmpty: Story = {
  args: { links: [] },
};
