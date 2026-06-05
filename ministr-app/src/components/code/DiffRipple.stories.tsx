import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ChangedSymbol, DiffImpact, ImpactedSymbol } from "../../lib/types";
import { DiffRipple } from "./DiffRipple";

/**
 * DiffRipple — a branch diff's blast radius as a ripple
 * (aaa-changes-ripple). The change sits at the risk-toned core; changed seed
 * symbols ring it (coloured by git-blame author); impacted symbols ripple out
 * on concentric rings by call-graph depth. The visual hero of the Changes lens.
 */

function changed(
  over: Partial<ChangedSymbol> & { name: string; file: string; line: number; last_author: string },
): ChangedSymbol {
  return { symbol_id: `sym-${over.name}`, kind: "function", authors: [{ name: over.last_author, lines: 20 }], ...over };
}

function impacted(over: Partial<ImpactedSymbol> & { name: string; file: string; depth: number }): ImpactedSymbol {
  return { symbol_id: `sym-${over.name}`, kind: "function", line: 1, ...over };
}

const RICH: DiffImpact = {
  range: "main..HEAD",
  changed_files: 3,
  changed_symbols: [
    changed({ name: "compute_impact", file: "ministr-core/src/service/code.rs", line: 325, last_author: "Alrik Olson" }),
    changed({ name: "ImpactResult", kind: "struct", file: "ministr-core/src/service/mod.rs", line: 231, last_author: "Alrik Olson" }),
    changed({ name: "diff_impact", file: "ministr-mcp/src/server/mod.rs", line: 137, last_author: "Priya Singh" }),
    changed({ name: "changed_lines", file: "ministr-mcp/src/server/mod.rs", line: 412, last_author: "Dana Vu" }),
  ],
  impacted_symbols: 14,
  impacted_files: 7,
  impacted_tests: 3,
  risk: "high",
  impacted: [
    ...Array.from({ length: 5 }, (_, i) => impacted({ name: `caller_d1_${i}`, file: `ministr-mcp/src/m${i}.rs`, depth: 1 })),
    ...Array.from({ length: 6 }, (_, i) => impacted({ name: `caller_d2_${i}`, file: `ministr-daemon/src/n${i}.rs`, depth: 2 })),
    ...Array.from({ length: 3 }, (_, i) => impacted({ name: `caller_d3_${i}`, file: `ministr-cli/src/c${i}.rs`, depth: 3 })),
  ],
};

const ISOLATED: DiffImpact = {
  range: "HEAD~1..HEAD",
  changed_files: 1,
  changed_symbols: [
    changed({ name: "format_range", file: "ministr-core/src/git/diff.rs", line: 88, last_author: "Alrik Olson" }),
  ],
  impacted_symbols: 0,
  impacted_files: 0,
  impacted_tests: 0,
  risk: "low",
  impacted: [],
};

const SINGLE_AUTHOR: DiffImpact = {
  range: "main..HEAD",
  changed_files: 1,
  changed_symbols: [
    changed({ name: "QueryService", kind: "struct", file: "ministr-core/src/service/query.rs", line: 42, last_author: "Alrik Olson" }),
    changed({ name: "survey", file: "ministr-core/src/service/query.rs", line: 120, last_author: "Alrik Olson" }),
  ],
  impacted_symbols: 4,
  impacted_files: 3,
  impacted_tests: 1,
  risk: "medium",
  impacted: [
    impacted({ name: "ask_corpus", file: "ministr-daemon/src/ask.rs", depth: 1 }),
    impacted({ name: "cmd_survey", file: "ministr-cli/src/commands.rs", depth: 1 }),
    impacted({ name: "MinistrServer::survey", file: "ministr-mcp/src/server/mod.rs", depth: 2 }),
    impacted({ name: "survey_eval", file: "ministr-core/tests/eval.rs", depth: 2 }),
  ],
};

const noop = () => {};

const meta = {
  title: "Code/DiffRipple",
  component: DiffRipple,
  parameters: { layout: "padded" },
  args: { onInspect: noop },
  decorators: [
    (Story) => (
      <div className="w-full max-w-[560px] bg-surface p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof DiffRipple>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A far-reaching, multi-author change — 14 impacted across 3 hops, high risk. */
export const Rich: Story = { args: { data: RICH } };

/** An isolated change — seed only, nothing ripples out. */
export const Isolated: Story = { args: { data: ISOLATED } };

/** A single-author change with a shallow ripple. */
export const SingleAuthor: Story = { args: { data: SINGLE_AUTHOR } };
