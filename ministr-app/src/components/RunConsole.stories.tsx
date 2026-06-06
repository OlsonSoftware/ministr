import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { within, userEvent, expect } from "storybook/test";
import { RunConsole, type ExecRun } from "./RunConsole";
import { withTauriMock } from "../../.storybook/tauri-mock";

/**
 * RunConsole — the recorded shell (`ministr_run`) made visible as a
 * command-deck run board.
 *
 * Each story renders one lifecycle state of the audit trail through the
 * `list_exec_runs` tauri-mock fixture: empty (front door), live (a run in
 * flight glows), success / failure / killed (finished runs go quiet, the
 * captured log expands with the severity left-gutter convention). All
 * fixtures are byte-deterministic: the clock is frozen via the `now` prop
 * so durations never drift under the gate.
 */

/** Frozen story clock — all fixture timestamps are relative to this. */
const NOW = 1_780_000_000_000;

function run(partial: Partial<ExecRun> & { run_id: string }): ExecRun {
  return {
    command: "cargo test -p ministr-core",
    cwd: "/work/ministr",
    session_id: "session-7f3a2b",
    corpus_id: null,
    env_fingerprint: "9c2f1ab4d0e57f31",
    started_at_ms: NOW - 95_000,
    finished_at_ms: NOW - 41_000,
    exit_code: 0,
    status: "exited",
    log: "",
    truncated: false,
    bytes_total: 18_432,
    ...partial,
  };
}

const SUCCESS_LOG = [
  "   Compiling ministr-core v0.6.0 (/work/ministr/ministr-core)",
  "    Finished `test` profile [unoptimized + debuginfo] target(s) in 42.18s",
  "     Running unittests src/lib.rs",
  "test result: ok. 1633 passed; 0 failed; 10 ignored",
].join("\n");

const FAIL_LOG = [
  "   Compiling ministr-core v0.6.0 (/work/ministr/ministr-core)",
  "error[E0308]: mismatched types",
  "  --> ministr-core/src/service/mod.rs:412:9",
  "   |",
  "412|         Ok(results)",
  "   |         ^^^^^^^^^^^ expected `Vec<Hit>`, found `Vec<Result>`",
  "warning: unused import: `std::fmt::Write`",
  "error: could not compile `ministr-core` (lib) due to 1 previous error",
].join("\n");

const HISTORY: ExecRun[] = [
  run({
    run_id: "run-1779999990000-4",
    command: "just validate",
    started_at_ms: NOW - 30_000,
    finished_at_ms: null,
    exit_code: null,
    status: "running",
    bytes_total: 0,
  }),
  run({
    run_id: "run-1779999980000-3",
    command: "cargo clippy --workspace --all-targets -- -D warnings",
    started_at_ms: NOW - 340_000,
    finished_at_ms: NOW - 251_000,
    exit_code: 1,
    log: FAIL_LOG,
    bytes_total: 84_120,
  }),
  run({
    run_id: "run-1779999970000-2",
    command: "cargo test -p ministr-daemon --test exec_engine",
    started_at_ms: NOW - 600_000,
    finished_at_ms: NOW - 560_000,
    log: SUCCESS_LOG,
  }),
  run({
    run_id: "run-1779999960000-1",
    command: "npm run build",
    started_at_ms: NOW - 900_000,
    finished_at_ms: NOW - 840_000,
    exit_code: null,
    status: "killed",
    log: "vite v6.0.3 building for production...\n…[output guard: 0 bytes dropped]…",
    bytes_total: 2_048,
  }),
  run({
    run_id: "run-1779999950000-0",
    command: "sleep 3600",
    started_at_ms: NOW - 1_500_000,
    finished_at_ms: NOW - 899_000,
    exit_code: null,
    status: "timed_out",
    log: "",
    bytes_total: 0,
  }),
];

function Frame({ children }: { children: ReactNode }) {
  return <div className="h-[560px] p-4 bg-surface">{children}</div>;
}

const meta = {
  title: "Surfaces/RunConsole",
  component: RunConsole,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof RunConsole>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Front door before any agent has executed a command. */
export const Empty: Story = {
  decorators: [withTauriMock({ list_exec_runs: [] })],
  render: () => (
    <Frame>
      <RunConsole now={NOW} />
    </Frame>
  ),
};

/** A run in flight: the medallion + card glow, the live pill counts it. */
export const LiveRun: Story = {
  decorators: [withTauriMock({ list_exec_runs: HISTORY })],
  render: () => (
    <Frame>
      <RunConsole now={NOW} />
    </Frame>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByText("1 live")).toBeInTheDocument();
    await expect(canvas.getByText("just validate")).toBeInTheDocument();
    // The live run expands into the honest "appears when finished" note,
    // not a fake tail.
    await userEvent.click(canvas.getByText("just validate"));
    await expect(
      await canvas.findByText(/appears\s+here when the command finishes/),
    ).toBeInTheDocument();
  },
};

/** A finished, green run expands into its captured log. */
export const FinishedSuccess: Story = {
  decorators: [
    withTauriMock({
      list_exec_runs: [HISTORY[2]],
    }),
  ],
  render: () => (
    <Frame>
      <RunConsole now={NOW} />
    </Frame>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const row = await canvas.findByText(
      "cargo test -p ministr-daemon --test exec_engine",
    );
    await userEvent.click(row);
    await expect(
      await canvas.findByText(/1633 passed/),
    ).toBeInTheDocument();
  },
};

/** A failed run: danger stripe on the row, error lines striped in the log. */
export const FinishedFailure: Story = {
  decorators: [
    withTauriMock({
      list_exec_runs: [HISTORY[1]],
    }),
  ],
  render: () => (
    <Frame>
      <RunConsole now={NOW} />
    </Frame>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByText("exit 1")).toBeInTheDocument();
    await userEvent.click(
      canvas.getByText(
        "cargo clippy --workspace --all-targets -- -D warnings",
      ),
    );
    await expect(
      await canvas.findByText(/mismatched types/),
    ).toBeInTheDocument();
  },
};

/** Killed + timed-out runs read from the warning tone dot, words stay AA. */
export const KilledAndTimedOut: Story = {
  decorators: [
    withTauriMock({
      list_exec_runs: [HISTORY[3], HISTORY[4]],
    }),
  ],
  render: () => (
    <Frame>
      <RunConsole now={NOW} />
    </Frame>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByText("killed")).toBeInTheDocument();
    await expect(canvas.getByText("timed out")).toBeInTheDocument();
  },
};
