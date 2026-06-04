import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ChangedSymbol, DiffImpact, ImpactedSymbol } from "../../lib/types";
import { ChangesMap } from "./ChangesMap";

/**
 * ChangesMap — the Explore "Changes" lens (aaa-explore-changes).
 * FL7's review / targeted-PR-context station: a branch DIFF as a first-class
 * object — WHAT changed (the symbols the range touched), WHO owns it (per-symbol
 * git blame), and WHAT IT CAN BREAK (the union blast radius over the ref graph).
 */

function changed(
  over: Partial<ChangedSymbol> & { name: string; file: string; line: number },
): ChangedSymbol {
  return {
    symbol_id: `sym-${over.name}`,
    kind: "function",
    authors: [],
    last_author: null,
    ...over,
  };
}

function impacted(over: Partial<ImpactedSymbol> & { name: string; file: string }): ImpactedSymbol {
  return {
    symbol_id: `sym-${over.name}`,
    kind: "function",
    line: 1,
    depth: 1,
    ...over,
  };
}

const RICH: DiffImpact = {
  range: "main..HEAD",
  changed_files: 3,
  changed_symbols: [
    changed({
      name: "compute_impact",
      kind: "function",
      file: "ministr-core/src/service/code.rs",
      line: 325,
      authors: [
        { name: "Alrik Olson", lines: 84 },
        { name: "Dana Vu", lines: 12 },
      ],
      last_author: "Alrik Olson",
    }),
    changed({
      name: "ImpactResult",
      kind: "struct",
      file: "ministr-core/src/service/mod.rs",
      line: 231,
      authors: [{ name: "Alrik Olson", lines: 22 }],
      last_author: "Alrik Olson",
    }),
    changed({
      name: "diff_impact",
      kind: "function",
      file: "ministr-mcp/src/server/mod.rs",
      line: 137,
      authors: [
        { name: "Alrik Olson", lines: 61 },
        { name: "Priya Singh", lines: 9 },
        { name: "Dana Vu", lines: 4 },
      ],
      last_author: "Priya Singh",
    }),
    changed({
      name: "changed_lines",
      kind: "function",
      file: "ministr-mcp/src/server/mod.rs",
      line: 412,
      authors: [{ name: "Priya Singh", lines: 31 }],
      last_author: "Priya Singh",
    }),
  ],
  impacted_symbols: 6,
  impacted_files: 4,
  impacted_tests: 2,
  risk: "medium",
  impacted: [
    impacted({ name: "impact", file: "ministr-mcp/src/server/mod.rs", line: 2573, depth: 1 }),
    impacted({ name: "Backend::impact", file: "ministr-mcp/src/backend/mod.rs", line: 709, depth: 1 }),
    impacted({ name: "lsp_parity_gate", file: "ministr-mcp/tests/lsp_parity.rs", line: 67, depth: 2 }),
    impacted({ name: "impact_response", file: "ministr-daemon/src/convert.rs", line: 117, depth: 2 }),
    impacted({ name: "fl7_diff_impact", file: "ministr-mcp/tests/diff_impact.rs", line: 20, depth: 2 }),
    impacted({ name: "cmd_impact", file: "ministr-cli/src/commands/impact.rs", line: 14, depth: 3 }),
  ],
};

const ISOLATED: DiffImpact = {
  range: "HEAD~1..HEAD",
  changed_files: 1,
  changed_symbols: [
    changed({
      name: "format_range",
      kind: "function",
      file: "ministr-core/src/git/diff.rs",
      line: 88,
      authors: [{ name: "Alrik Olson", lines: 6 }],
      last_author: "Alrik Olson",
    }),
  ],
  impacted_symbols: 0,
  impacted_files: 0,
  impacted_tests: 0,
  risk: "low",
  impacted: [],
};

const EMPTY: DiffImpact = {
  range: "main..HEAD",
  changed_files: 0,
  changed_symbols: [],
  impacted_symbols: 0,
  impacted_files: 0,
  impacted_tests: 0,
  risk: "low",
  impacted: [],
};

const noop = () => {};

const meta = {
  title: "Code/ChangesMap",
  component: ChangesMap,
  parameters: { layout: "fullscreen" },
  args: {
    range: "main..HEAD",
    onRangeChange: noop,
    onRun: noop,
    onInspect: noop,
    onOpenFile: noop,
  },
  decorators: [
    (Story) => (
      <div className="h-[820px] w-full bg-bg">
        <div className="mx-auto h-full max-w-4xl border-x border-border bg-surface">
          <Story />
        </div>
      </div>
    ),
  ],
} satisfies Meta<typeof ChangesMap>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A branch diff with changed symbols (+ blame) and a union blast radius. */
export const Rich: Story = { args: { data: RICH } };

/** An isolated change — nothing references the changed symbol. */
export const Isolated: Story = { args: { data: ISOLATED } };

/** The range touched no indexed symbols (config/docs only, or empty). */
export const NothingIndexed: Story = { args: { data: EMPTY } };

/** First mount — invite the reviewer to run a range. */
export const Idle: Story = { args: { data: null } };

/** Resolving the diff. */
export const Loading: Story = { args: { data: null, loading: true } };

/** Not a git checkout — no branch diff to review. */
export const NoRepo: Story = { args: { data: null, hasRepo: false } };

/** A bad range / git error. */
export const Errored: Story = {
  args: { data: null, error: "git range 'nope..HEAD': fatal: bad revision 'nope..HEAD'" },
};
