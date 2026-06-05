import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ImpactedSymbol, SymbolImpact as SymbolImpactData } from "../../lib/types";
import { BlastRadiusMap } from "./BlastRadiusMap";

/**
 * BlastRadiusMap — a symbol's call graph as a picture (aaa-symbol-blast-radius-map).
 * Callers fan into the risk-toned core (the blast radius); callees fan out
 * below; edge weight encodes call-graph depth. The visual hero of the symbol
 * inspector's Impact facet.
 */

function node(
  over: Partial<ImpactedSymbol> & { name: string; file: string; depth: number },
): ImpactedSymbol {
  return { symbol_id: `sym-${over.name}`, kind: "function", line: 1, ...over };
}

const RICH: SymbolImpactData = {
  incoming: [
    node({ name: "compute_impact", file: "ministr-core/src/service/code.rs", depth: 1 }),
    node({ name: "Backend::impact", file: "ministr-mcp/src/backend/mod.rs", depth: 1 }),
    node({ name: "MinistrServer::impact", file: "ministr-mcp/src/server/mod.rs", depth: 2 }),
    node({ name: "cmd_impact", file: "ministr-cli/src/commands/impact.rs", depth: 3 }),
  ],
  incoming_symbols: 4,
  incoming_files: 4,
  incoming_tests: 1,
  risk: "medium",
  outgoing: [
    node({ name: "query_refs", file: "ministr-core/src/storage/sqlite.rs", depth: 1 }),
    node({ name: "get_symbol", file: "ministr-core/src/storage/sqlite.rs", depth: 1 }),
    node({ name: "compute_risk", file: "ministr-core/src/service/code.rs", depth: 2 }),
  ],
  outgoing_symbols: 3,
  tests: [
    node({ name: "lsp_parity_gate", file: "ministr-mcp/tests/lsp_parity.rs", depth: 2 }),
    node({ name: "impact_incoming_outgoing", file: "ministr-core/tests/impact.rs", depth: 1 }),
  ],
};

/** High risk, many callers (exercises the +N overflow marker), no coverage. */
const HIGH_RISK: SymbolImpactData = {
  incoming: Array.from({ length: 9 }, (_, i) =>
    node({ name: `caller_${i + 1}`, file: `ministr-core/src/m${i}.rs`, depth: (i % 3) + 1 }),
  ),
  incoming_symbols: 9,
  incoming_files: 9,
  incoming_tests: 0,
  risk: "high",
  outgoing: [
    node({ name: "store_write", file: "ministr-core/src/storage/sqlite.rs", depth: 1 }),
  ],
  outgoing_symbols: 1,
  tests: [],
};

const UNCOVERED: SymbolImpactData = {
  incoming: [node({ name: "render_glance", file: "ministr-app/src/components/code/ChangesMap.tsx", depth: 1 })],
  incoming_symbols: 1,
  incoming_files: 1,
  incoming_tests: 0,
  risk: "low",
  outgoing: [node({ name: "fileTail", file: "ministr-app/src/components/code/ChangesMap.tsx", depth: 1 })],
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
  title: "Code/BlastRadiusMap",
  component: BlastRadiusMap,
  parameters: { layout: "padded" },
  args: { onOpenSymbol: noop },
  decorators: [
    (Story) => (
      <div className="w-full max-w-[440px] rounded-md border border-border-soft bg-surface p-3">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof BlastRadiusMap>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A well-connected symbol: callers in, callees out, covered. */
export const Rich: Story = { args: { data: RICH } };

/** High risk + 9 callers (the +N overflow marker) + no coverage. */
export const HighRiskOverflow: Story = { args: { data: HIGH_RISK } };

/** One caller, one callee, no tests — the low-risk coverage gap. */
export const Uncovered: Story = { args: { data: UNCOVERED } };

/** A leaf / entry point — nothing in, nothing out. The lone core. */
export const Leaf: Story = { args: { data: LEAF } };
