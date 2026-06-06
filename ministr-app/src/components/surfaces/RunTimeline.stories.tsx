import type { Meta, StoryObj } from "@storybook/react-vite";
import { within, expect } from "storybook/test";
import { RunTimeline } from "./RunTimeline";
import type { ExecRun } from "../RunConsole";

/**
 * RunTimeline — the exec lane's bespoke temporal viz: recorded runs as
 * duration bars on a shared time axis (ActivityPulse idiom family),
 * status-toned, command labels in a fixed fitLabel gutter, inside the
 * shared VizFrame. Byte-deterministic: the clock is the frozen `now`
 * prop, fixtures carry fixed timestamps.
 */

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

const SESSION: ExecRun[] = [
  run({
    run_id: "run-1779999990000-4",
    command: "just validate",
    started_at_ms: NOW - 30_000,
    finished_at_ms: null,
    exit_code: null,
    status: "running",
  }),
  run({
    run_id: "run-1779999980000-3",
    command:
      "cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic",
    started_at_ms: NOW - 340_000,
    finished_at_ms: NOW - 251_000,
    exit_code: 1,
  }),
  run({
    run_id: "run-1779999970000-2",
    command: "cargo test -p ministr-daemon --test exec_engine",
    started_at_ms: NOW - 600_000,
    finished_at_ms: NOW - 560_000,
  }),
  run({
    run_id: "run-1779999960000-1",
    command: "npm run build",
    started_at_ms: NOW - 900_000,
    finished_at_ms: NOW - 840_000,
    exit_code: null,
    status: "killed",
  }),
  run({
    run_id: "run-1779999950000-0",
    command: "sleep 3600",
    started_at_ms: NOW - 1_500_000,
    finished_at_ms: NOW - 899_000,
    exit_code: null,
    status: "timed_out",
  }),
];

const meta = {
  title: "Surfaces/RunTimeline",
  component: RunTimeline,
  parameters: { layout: "padded" },
} satisfies Meta<typeof RunTimeline>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A working session: live run at the now-edge, a failure, a kill, a timeout. */
export const Session: Story = {
  args: { runs: SESSION, now: NOW },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const svg = canvasElement.querySelector("svg[role='img']");
    await expect(svg).toBeTruthy();
    await expect(svg!.getAttribute("aria-label")).toContain("5 runs");
    // The over-long clippy command is truncated to the gutter.
    await expect(canvas.getByText(/cargo clippy .*…/)).toBeInTheDocument();
    // Readout carries the honest counts.
    await expect(canvas.getByText("1 failed")).toBeInTheDocument();
  },
};

/** Instant commands: the epsilon window keeps bars visible, no NaN x. */
export const InstantBurst: Story = {
  args: {
    runs: [0, 1, 2].map((i) =>
      run({
        run_id: `run-177999999000${i}-${i}`,
        command: `echo step-${i}`,
        started_at_ms: NOW - 1000,
        finished_at_ms: NOW - 1000,
        bytes_total: 8,
      }),
    ),
    now: NOW,
  },
  play: async ({ canvasElement }) => {
    const rects = canvasElement.querySelectorAll("svg rect");
    await expect(rects.length).toBe(3);
    for (const r of rects) {
      const w = Number(r.getAttribute("width"));
      await expect(Number.isFinite(w) && w >= 2).toBe(true);
    }
  },
};

/** Every run failed — the danger story reads from tone alone. */
export const AllFailed: Story = {
  args: {
    runs: [0, 1, 2, 3].map((i) =>
      run({
        run_id: `run-17799999${8 - i}000-${i}`,
        command: `cargo test --test suite_${i}`,
        started_at_ms: NOW - (i + 1) * 120_000,
        finished_at_ms: NOW - (i + 1) * 120_000 + 60_000,
        exit_code: 101,
      }),
    ),
    now: NOW,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("4 failed")).toBeInTheDocument();
  },
};

/** More runs than lanes: the cap shows in the readout (no silent drop). */
export const ManyRuns: Story = {
  args: {
    runs: Array.from({ length: 14 }, (_, i) =>
      run({
        run_id: `run-17799${99 - i}00000-${i}`,
        command: `task-${i}`,
        started_at_ms: NOW - (i + 1) * 60_000,
        finished_at_ms: NOW - (i + 1) * 60_000 + 30_000,
      }),
    ),
    now: NOW,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("10/14 runs")).toBeInTheDocument();
  },
};
