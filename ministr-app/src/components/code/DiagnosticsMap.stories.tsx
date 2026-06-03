import type { Meta, StoryObj } from "@storybook/react-vite";
import type { Diagnostic, DiagnosticSeverity } from "../../lib/types";
import { DiagnosticsMap } from "./DiagnosticsMap";

/**
 * DiagnosticsMap — the Explore "Diagnostics" lens (aaa-explore-diagnostics).
 * FL5's verify stage: the project's own toolchain (cargo / tsc / eslint / ruff
 * / go vet / …) normalised to one shape, errors-first by file. Language-agnostic
 * — a TypeScript error and a Rust error render identically.
 */

function diag(
  over: Partial<Diagnostic> & {
    file: string;
    message: string;
    severity: DiagnosticSeverity;
  },
): Diagnostic {
  return {
    line_start: 42,
    col_start: 5,
    line_end: 42,
    col_end: 12,
    code: null,
    source: "cargo",
    symbol_id: null,
    ...over,
  };
}

const DIAGS: Diagnostic[] = [
  diag({
    severity: "error",
    code: "E0599",
    source: "cargo",
    message: "no method named `foo` found for struct `Bar` in the current scope",
    file: "ministr-core/src/service/code.rs",
    line_start: 212,
    symbol_id: "sym-service::code::Bar::run",
  }),
  diag({
    severity: "error",
    code: "E0308",
    source: "cargo",
    message: "mismatched types: expected `String`, found `&str`",
    file: "ministr-core/src/service/code.rs",
    line_start: 240,
  }),
  diag({
    severity: "warning",
    code: "unused_variables",
    source: "cargo",
    message: "unused variable: `ctx` — prefix with an underscore to silence",
    file: "ministr-core/src/service/code.rs",
    line_start: 88,
    symbol_id: "sym-service::code::helper",
  }),
  diag({
    severity: "error",
    code: "TS2345",
    source: "tsc",
    message:
      "Argument of type 'string' is not assignable to parameter of type 'number'",
    file: "ministr-app/src/components/code/CodeViewer.tsx",
    line_start: 51,
  }),
  diag({
    severity: "warning",
    code: "no-unused-vars",
    source: "eslint",
    message: "'scheme' is assigned a value but never used",
    file: "ministr-app/src/components/code/CodeViewer.tsx",
    line_start: 12,
  }),
  diag({
    severity: "warning",
    code: "F401",
    source: "ruff",
    message: "`os` imported but unused; remove the import",
    file: "scripts/ci/ci.py",
    line_start: 3,
  }),
  diag({
    severity: "info",
    code: null,
    source: "go vet",
    message: "result of fmt.Sprintf call not used",
    file: "pkg/server/main.go",
    line_start: 120,
  }),
];

const noop = () => {};

const meta = {
  title: "Code/DiagnosticsMap",
  component: DiagnosticsMap,
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
} satisfies Meta<typeof DiagnosticsMap>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Cross-language findings (cargo / tsc / eslint / ruff / go vet), errors-first
 *  by file — the verify stage made visible. */
export const Rich: Story = { args: { diagnostics: DIAGS } };

/** A clean build — the toolchain reports nothing (or none detected). */
export const Empty: Story = { args: { diagnostics: [] } };

/** First load — running the toolchain. */
export const Loading: Story = { args: { diagnostics: [], loading: true } };
