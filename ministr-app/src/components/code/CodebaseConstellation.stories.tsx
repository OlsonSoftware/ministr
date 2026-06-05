import type { Meta, StoryObj } from "@storybook/react-vite";
import type { FileInfo } from "../../lib/types";
import { CodebaseConstellation } from "./CodebaseConstellation";

/**
 * CodebaseConstellation — the shape of the codebase (aaa-codebase-constellation).
 * Indexed files grouped into top-level modules, each a bubble sized by index
 * mass and deterministically circle-packed. The Observatory's structure beat.
 */

function file(path: string, section_count: number): FileInfo {
  return { path, content_hash: "h", mtime_ns: 0, section_count };
}

/** Spread `n` files across a module dir, with descending section counts. */
function mod(dir: string, n: number, base: number): FileInfo[] {
  return Array.from({ length: n }, (_, i) =>
    file(`${dir}/f${i}.rs`, Math.max(1, base - i * 2)),
  );
}

// A monorepo of crates — the rich, multi-module case.
const RICH: FileInfo[] = [
  ...mod("ministr-core/src", 40, 60),
  ...mod("ministr-app/src/components", 30, 28),
  ...mod("ministr-daemon/src", 18, 24),
  ...mod("ministr-mcp/src", 16, 20),
  ...mod("ministr-api/src", 14, 18),
  ...mod("ministr-cli/src", 9, 14),
  ...mod("eval/corpus", 6, 8),
  ...mod("docs", 5, 4),
  ...mod("scripts/ci", 4, 3),
  ...mod("web/lib", 7, 6),
];

// A src-rooted single-root app — the adaptive regroup should descend past src/.
const SRC_ROOTED: FileInfo[] = [
  ...mod("src/components", 24, 30),
  ...mod("src/lib", 14, 16),
  ...mod("src/hooks", 8, 9),
  ...mod("src/pages", 10, 12),
  ...mod("src/styles", 3, 4),
];

// Just a few modules.
const FEW: FileInfo[] = [
  ...mod("server", 12, 20),
  ...mod("client", 9, 14),
  ...mod("shared", 4, 6),
];

const noop = () => {};

const meta = {
  title: "Code/CodebaseConstellation",
  component: CodebaseConstellation,
  parameters: { layout: "padded" },
  args: { onOpen: noop },
  decorators: [
    (Story) => (
      <div className="w-full max-w-[680px] bg-surface p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof CodebaseConstellation>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A monorepo of crates — a dominant core + a field of smaller modules. */
export const Rich: Story = { args: { files: RICH } };

/** A src-rooted app — the grouping descends past `src/` to read structure. */
export const SrcRooted: Story = { args: { files: SRC_ROOTED } };

/** A handful of modules. */
export const Few: Story = { args: { files: FEW } };
