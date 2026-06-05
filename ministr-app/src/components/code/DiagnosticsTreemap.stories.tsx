import type { Meta, StoryObj } from "@storybook/react-vite";
import type { Diagnostic, DiagnosticSeverity } from "../../lib/types";
import { DiagnosticsTreemap } from "./DiagnosticsTreemap";

/**
 * DiagnosticsTreemap — the toolchain findings as a hot-spot map
 * (aaa-diagnostics-treemap). A bespoke squarified treemap: each file is a
 * rectangle sized by its finding count and coloured by its worst severity
 * (error=danger / warning=warning / info=accent / hint=muted). The visual hero
 * of the Diagnostics lens, above the grouped list.
 */

function diag(
  over: Partial<Diagnostic> & { file: string; severity: DiagnosticSeverity },
): Diagnostic {
  return {
    line_start: 42,
    col_start: 1,
    line_end: 42,
    col_end: 8,
    code: null,
    source: "cargo",
    symbol_id: null,
    message: "finding",
    ...over,
  };
}

/** Spread `n` findings of a severity across one file. */
function spread(file: string, severity: DiagnosticSeverity, n: number): Diagnostic[] {
  return Array.from({ length: n }, (_, i) => diag({ file, severity, line_start: 10 + i }));
}

// A realistic mix — a couple of error hot-spots, several warning files, a long
// tail (to exercise the "+N files" tile).
const RICH: Diagnostic[] = [
  ...spread("ministr-core/src/service/code.rs", "error", 9),
  ...spread("ministr-core/src/service/code.rs", "warning", 3),
  ...spread("ministr-app/src/components/code/CodeViewer.tsx", "error", 4),
  ...spread("ministr-app/src/components/code/CodeViewer.tsx", "warning", 2),
  ...spread("ministr-daemon/src/daemon.rs", "warning", 6),
  ...spread("ministr-mcp/src/server/mod.rs", "warning", 4),
  ...spread("scripts/ci/ci.py", "warning", 3),
  ...spread("pkg/server/main.go", "info", 5),
  ...spread("ministr-api/src/client.rs", "info", 2),
  ...spread("ministr-cli/src/commands.rs", "hint", 3),
  ...Array.from({ length: 18 }, (_, i) =>
    diag({ file: `crate/src/mod_${i}.rs`, severity: i % 2 ? "warning" : "info", line_start: 5 }),
  ),
];

const ERRORS_ONLY: Diagnostic[] = [
  ...spread("ministr-core/src/service/code.rs", "error", 6),
  ...spread("ministr-app/src/components/code/CodeViewer.tsx", "error", 3),
  ...spread("ministr-daemon/src/daemon.rs", "error", 2),
  ...spread("ministr-cli/src/commands.rs", "error", 1),
];

const SINGLE: Diagnostic[] = [
  ...spread("ministr-core/src/service/code.rs", "error", 2),
  ...spread("ministr-core/src/service/code.rs", "warning", 1),
];

const noop = () => {};

const meta = {
  title: "Code/DiagnosticsTreemap",
  component: DiagnosticsTreemap,
  parameters: { layout: "padded" },
  args: { onOpenFile: noop },
  decorators: [
    (Story) => (
      <div className="w-full max-w-[640px] bg-surface p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof DiagnosticsTreemap>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Many files, mixed severity, a long tail folded into "+N files". */
export const Rich: Story = { args: { diagnostics: RICH } };

/** Errors only (the severity filter applied) — every tile is danger-toned. */
export const ErrorsOnly: Story = { args: { diagnostics: ERRORS_ONLY } };

/** A single hot file — one big tile. */
export const SingleFile: Story = { args: { diagnostics: SINGLE } };
