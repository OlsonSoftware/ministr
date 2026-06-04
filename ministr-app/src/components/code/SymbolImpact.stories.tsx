import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ImpactedSymbol, SymbolImpact as SymbolImpactData } from "../../lib/types";
import { SymbolImpact } from "./SymbolImpact";

/**
 * SymbolImpact — the symbol inspector's call-hierarchy + coverage facet
 * (aaa-symbol-impact). Surfaces FL3 (who calls / what it calls) and FL6 (which
 * tests cover it) as a first-class facet of the symbol object.
 */

function node(over: Partial<ImpactedSymbol> & { name: string; file: string; depth: number }): ImpactedSymbol {
  return { symbol_id: `sym-${over.name}`, kind: "function", line: 1, ...over };
}

const RICH: SymbolImpactData = {
  incoming: [
    node({ name: "compute_impact", kind: "function", file: "ministr-core/src/service/code.rs", depth: 1 }),
    node({ name: "Backend::impact", kind: "function", file: "ministr-mcp/src/backend/mod.rs", depth: 1 }),
    node({ name: "MinistrServer::impact", kind: "function", file: "ministr-mcp/src/server/mod.rs", depth: 2 }),
    node({ name: "cmd_impact", kind: "function", file: "ministr-cli/src/commands/impact.rs", depth: 3 }),
  ],
  incoming_symbols: 4,
  incoming_files: 4,
  incoming_tests: 1,
  risk: "medium",
  outgoing: [
    node({ name: "query_refs", kind: "function", file: "ministr-core/src/storage/sqlite.rs", depth: 1 }),
    node({ name: "get_symbol", kind: "function", file: "ministr-core/src/storage/sqlite.rs", depth: 1 }),
    node({ name: "compute_risk", kind: "function", file: "ministr-core/src/service/code.rs", depth: 2 }),
  ],
  outgoing_symbols: 3,
  tests: [
    node({ name: "lsp_parity_gate", kind: "function", file: "ministr-mcp/tests/lsp_parity.rs", depth: 2 }),
    node({ name: "impact_incoming_outgoing", kind: "function", file: "ministr-core/tests/impact.rs", depth: 1 }),
  ],
};

const UNCOVERED: SymbolImpactData = {
  incoming: [
    node({ name: "render_glance", kind: "function", file: "ministr-app/src/components/code/ChangesMap.tsx", depth: 1 }),
  ],
  incoming_symbols: 1,
  incoming_files: 1,
  incoming_tests: 0,
  risk: "low",
  outgoing: [
    node({ name: "fileTail", kind: "function", file: "ministr-app/src/components/code/ChangesMap.tsx", depth: 1 }),
  ],
  outgoing_symbols: 1,
  tests: [],
};

const LEAF: SymbolImpactData = {
  incoming: [],
  incoming_symbols: 0,
  incoming_files: 0,
  incoming_tests: 0,
  risk: "low",
  outgoing: [],
  outgoing_symbols: 0,
  tests: [],
};

const noop = () => {};

const meta = {
  title: "Code/SymbolImpact",
  component: SymbolImpact,
  parameters: { layout: "padded" },
  args: { onOpenSymbol: noop },
  decorators: [
    (Story) => (
      <div className="w-full max-w-[720px] bg-surface p-5">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SymbolImpact>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A well-connected symbol: callers (with risk), callees, and covering tests. */
export const Rich: Story = { args: { data: RICH } };

/** A symbol with callers but NO tests — the coverage-gap warning. */
export const Uncovered: Story = { args: { data: UNCOVERED } };

/** A leaf / entry point — nothing in any lane. */
export const Leaf: Story = { args: { data: LEAF } };

/** Tracing the call graph. */
export const Loading: Story = { args: { data: null, loading: true } };
