import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { within, userEvent, expect } from "storybook/test";
import { LogViewer } from "./LogViewer";
import { withTauriMock } from "../../.storybook/tauri-mock";

/**
 * LogViewer — the daemon tail reborn as a command-deck TELEMETRY CONSOLE.
 *
 * A glowing telemetry medallion + live/paused status lead the header; a bespoke
 * severity-banded LogRateRibbon (info/warn/error histogram) sits above the
 * filterable tail. All states render from the tauri-mock `read_logs` fixture —
 * this is the surface's first story (it was a Storybook blind spot), so axe now
 * gates it on every run.
 */

// ── Fixture builder — deterministic, realistic ministr daemon lines. ─────────

type Lvl = "INFO" | "WARN" | "ERROR";

function ts(i: number): string {
  // A stable fake wall-clock that advances ~0.4s per line. No Date.now() so the
  // story is byte-deterministic for the gate.
  const base = 18_000 + i * 400; // ms past 14:30:00
  const totalSec = Math.floor(base / 1000);
  const mm = 30 + Math.floor(totalSec / 60);
  const ss = totalSec % 60;
  const ms = base % 1000;
  return `2026-06-05T14:${String(mm).padStart(2, "0")}:${String(ss).padStart(2, "0")}.${String(ms).padStart(3, "0")}Z`;
}

function line(i: number, lvl: Lvl, msg: string): string {
  return `${ts(i)}  ${lvl}  ${msg}`;
}

const INFO_MSGS = [
  "ingestion::pipeline indexed ministr-core/src/service/code.rs (412 sections)",
  "embedding::jina batch of 64 → 64 vectors in 38ms",
  "query::survey session-7f3a2b served 8 results from cache (hit)",
  "watcher::fs change detected: ministr-app/src/components/LogViewer.tsx",
  "daemon::uds accepted connection from corpus-9a1c4e (claude-code)",
  "store::sqlite upsert 128 sections, 0 conflicts",
  "ref-graph resolved 1_204 edges across 3 languages",
  "mcp::survey corpus=ministr top_k=8 latency=11ms",
  "compress::section evicted 3 cold sections (saved 4_812 tokens)",
  "bridge::pyo3 linked 14 cross-language endpoints",
];

function richLogs(): string[] {
  const out: string[] = [];
  let i = 0;
  // A long quiet-ish info stream with periodic warnings…
  for (let k = 0; k < 110; k++, i++) {
    if (k % 17 === 8) {
      out.push(
        line(
          i,
          "WARN",
          `embedding::jina retrying batch ${k} after a 503 (attempt 2/3)`,
        ),
      );
    } else {
      out.push(line(i, "INFO", INFO_MSGS[k % INFO_MSGS.length]));
    }
  }
  // …then a burst of errors near the newest edge (the red spike).
  for (let k = 0; k < 7; k++, i++) {
    out.push(
      line(
        i,
        "ERROR",
        `query::survey corpus-9a1c4e degenerate result: zero-vector section content_id=sec_${k}d4f`,
      ),
    );
  }
  // A couple of trailing info lines so recovery is visible.
  for (let k = 0; k < 4; k++, i++) {
    out.push(line(i, "INFO", INFO_MSGS[k % INFO_MSGS.length]));
  }
  return out;
}

const QUIET_LOGS: string[] = Array.from({ length: 9 }, (_, k) =>
  line(k, "INFO", INFO_MSGS[k % INFO_MSGS.length]),
);

const NO_LOG: string[] = ["No log file found at ~/.ministr/ministr.log"];

// ── Frame — a bounded developer-panel-sized container. ───────────────────────

function Frame({ children }: { children: ReactNode }) {
  return (
    <div className="h-[680px] bg-surface p-5">
      <div className="h-full">{children}</div>
    </div>
  );
}

const meta = {
  title: "Surfaces/LogViewer",
  component: LogViewer,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof LogViewer>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Live tail: a long info stream with periodic warnings and a fresh burst of
 *  errors at the newest edge — the ribbon shows the red spike, the medallion
 *  glows, the status reads LIVE. */
export const Default: Story = {
  decorators: [withTauriMock({ read_logs: () => richLogs() })],
  render: () => (
    <Frame>
      <LogViewer />
    </Frame>
  ),
};

/** A calm info-only stream — the ribbon reads as a quiet neutral floor with no
 *  warning/error bands; the vital readout shows em-dashes. */
export const Quiet: Story = {
  decorators: [withTauriMock({ read_logs: () => QUIET_LOGS })],
  render: () => (
    <Frame>
      <LogViewer />
    </Frame>
  ),
};

/** No log file yet — the ribbon hides, the medallion is quiet, and the body
 *  shows the guidance empty state. */
export const NoLogs: Story = {
  decorators: [withTauriMock({ read_logs: () => NO_LOG })],
  render: () => (
    <Frame>
      <LogViewer />
    </Frame>
  ),
};

/** Paused feed — clicking PAUSE freezes the tail and flips the status pill to a
 *  warning-toned PAUSED. Exercises the live→paused medallion/pill transition. */
export const Paused: Story = {
  decorators: [withTauriMock({ read_logs: () => richLogs() })],
  render: () => (
    <Frame>
      <LogViewer />
    </Frame>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const pause = await canvas.findByRole("button", { name: /^pause$/i });
    await userEvent.click(pause);
    await expect(canvas.getByText("paused")).toBeInTheDocument();
  },
};
