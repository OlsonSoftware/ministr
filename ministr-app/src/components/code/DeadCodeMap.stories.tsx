import type { Meta, StoryObj } from "@storybook/react-vite";
import type { DeadSymbol } from "../../lib/types";
import { DeadCodeMap } from "./DeadCodeMap";

/**
 * DeadCodeMap — the Explore "Unused" lens (aaa-explore-deadcode). Zero-reference
 * symbols grouped by file, ranked by reclaimable lines; inspect to confirm
 * before deleting.
 */

function dead(
  over: Partial<DeadSymbol> & { name: string; file: string },
): DeadSymbol {
  return {
    symbol_id: `sym-${over.file}::${over.name}`,
    kind: "function",
    visibility: "pub",
    line: 42,
    lines: 14,
    ...over,
  };
}

const SYMBOLS: DeadSymbol[] = [
  dead({ name: "legacy_migrate_v1", file: "ministr-core/src/storage/migrate.rs", kind: "function", line: 88, lines: 64 }),
  dead({ name: "OldSessionShape", file: "ministr-core/src/storage/migrate.rs", kind: "struct", line: 12, lines: 22, visibility: "pub(crate)" }),
  dead({ name: "unused_helper", file: "ministr-core/src/storage/migrate.rs", kind: "function", line: 160, lines: 9, visibility: "fn" }),
  dead({ name: "ExperimentalReranker", file: "ministr-core/src/rerank/experimental.rs", kind: "struct", line: 30, lines: 120 }),
  dead({ name: "score_v2", file: "ministr-core/src/rerank/experimental.rs", kind: "function", line: 158, lines: 41 }),
  dead({ name: "DeprecatedFlag", file: "ministr-api/src/config.rs", kind: "enum", line: 210, lines: 8, visibility: "pub" }),
  dead({ name: "stub_only_path", file: "ministr-cli/src/commands.rs", kind: "function", line: 1990, lines: 6, visibility: "fn" }),
];

const noop = () => {};

const meta = {
  title: "Code/DeadCodeMap",
  component: DeadCodeMap,
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
} satisfies Meta<typeof DeadCodeMap>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Several candidates across files + kinds, ranked by reclaimable lines. */
export const Rich: Story = { args: { symbols: SYMBOLS } };

/** A clean reference graph — nothing to prune. */
export const Empty: Story = { args: { symbols: [] } };

/** First load — tracing the reference graph. */
export const Loading: Story = { args: { symbols: [], loading: true } };
