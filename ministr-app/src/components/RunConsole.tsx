import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Terminal } from "@/components/ui/icons";
import { StatusDot } from "./ui/status-dot";
import { RunTimeline } from "./surfaces/RunTimeline";
import type { Tone } from "../lib/status";
import { cn } from "../lib/utils";

/**
 * RunConsole — the recorded shell, made visible.
 *
 * Every `ministr_run` execution persists an audit record (command, cwd,
 * session, timestamps, exit code, captured log) in
 * `~/.ministr/exec_runs.db`. This command-deck surface reads that trail
 * via the `list_exec_runs` IPC: a glowing terminal medallion + live pill
 * lead the header; the body is a run board where active runs glow and
 * finished runs go quiet, each expandable into its captured log with a
 * severity left-gutter (the LogViewer convention — tone on NON-text so
 * message text stays AA).
 *
 * Honest limits (cross-process): the run engine lives in the MCP server
 * process, so this surface is read-only — no kill control, and a live
 * run's output appears when it finishes (the engine persists the log at
 * exit). Both lift when the engine moves into the daemon.
 */

export type ExecRunStatus = "running" | "exited" | "killed" | "timed_out";

export interface ExecRun {
  run_id: string;
  command: string;
  cwd: string;
  session_id: string | null;
  corpus_id: string | null;
  env_fingerprint: string;
  started_at_ms: number;
  finished_at_ms: number | null;
  exit_code: number | null;
  status: ExecRunStatus;
  log: string;
  truncated: boolean;
  bytes_total: number;
}

/** Poll cadence for the run board. */
const POLL_MS = 2000;

/** Lifecycle → presentation: tone (NON-text), label, card treatment. */
function runPresentation(run: ExecRun): {
  tone: Tone;
  label: string;
  live: boolean;
} {
  switch (run.status) {
    case "running":
      return { tone: "accent", label: "running", live: true };
    case "killed":
      return { tone: "warning", label: "killed", live: false };
    case "timed_out":
      return { tone: "warning", label: "timed out", live: false };
    case "exited":
      return run.exit_code === 0
        ? { tone: "success", label: "exit 0", live: false }
        : { tone: "danger", label: `exit ${run.exit_code ?? "?"}`, live: false };
  }
}

function formatDuration(run: ExecRun, now: number): string {
  const end = run.finished_at_ms ?? now;
  const ms = Math.max(0, end - run.started_at_ms);
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const m = Math.floor(ms / 60_000);
  const s = Math.round((ms % 60_000) / 1000);
  return `${m}m ${s}s`;
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/** Severity classification for captured log lines (LogViewer convention). */
function lineLevel(line: string): "error" | "warn" | "info" {
  const upper = line.toUpperCase();
  if (
    upper.includes("ERROR") ||
    upper.includes("PANICKED") ||
    upper.includes("FATAL") ||
    upper.includes("FAILED")
  ) {
    return "error";
  }
  if (upper.includes("WARN")) return "warn";
  return "info";
}

export function RunConsole({ now }: { now?: number }) {
  const [runs, setRuns] = useState<ExecRun[] | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [clock, setClock] = useState(() => now ?? Date.now());

  // Poll the audit trail. `now` (stories/tests) freezes the clock so
  // durations render deterministically.
  useEffect(() => {
    let cancelled = false;
    async function poll() {
      try {
        const fetched = await invoke<ExecRun[] | null>("list_exec_runs", {
          limit: 100,
        });
        if (!cancelled) setRuns(fetched ?? []);
      } catch {
        if (!cancelled) setRuns([]);
      }
      if (!cancelled && now === undefined) setClock(Date.now());
    }
    poll();
    const id = setInterval(poll, POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [now]);

  const list = useMemo(() => runs ?? [], [runs]);
  const activeCount = list.filter((r) => r.status === "running").length;
  const failedCount = list.filter(
    (r) => r.status === "exited" && r.exit_code !== 0,
  ).length;
  const isLive = activeCount > 0;

  return (
    <div className="flex flex-col h-full gap-3 min-h-0">
      {/* Command-deck identity header — terminal medallion glows while a
          run is live, goes quiet when the board is idle. */}
      <header className="flex flex-wrap items-center justify-between gap-3 shrink-0">
        <div className="flex min-w-0 items-center gap-3">
          <span
            aria-hidden
            className={cn(
              "relative grid h-11 w-11 shrink-0 place-items-center rounded-xl border bg-surface-overlay",
              isLive
                ? "border-accent/50 text-accent shadow-[var(--glow-soft)]"
                : "border-border text-text-muted",
            )}
          >
            <Terminal className="h-[18px] w-[18px]" strokeWidth={2} />
          </span>
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <span className="text-[15px] font-semibold text-text font-sans">
                Runs
              </span>
              {isLive && (
                <span className="inline-flex items-center gap-1.5 rounded-full border border-accent/40 bg-accent/10 px-2 py-0.5 font-mono text-mono-micro font-medium uppercase tracking-[0.06em] text-text">
                  <StatusDot tone="accent" pulse="live" />
                  {activeCount} live
                </span>
              )}
            </div>
            <div className="font-mono text-mono-mini text-text-dim">
              Recorded shell · ministr_run
            </div>
          </div>
        </div>
      </header>

      {/* Status transitions are announced without stealing focus. */}
      <p aria-live="polite" className="sr-only">
        {activeCount === 0
          ? "No runs active"
          : `${activeCount} run${activeCount === 1 ? "" : "s"} active`}
      </p>

      {/* Temporal shape of the session — duration bars on a time axis.
          Needs at least two runs to be a timeline rather than a bar. */}
      {list.length >= 2 && (
        <RunTimeline runs={list} now={clock} className="shrink-0" />
      )}

      {/* Run board */}
      <div className="relative flex-1 overflow-y-auto rounded-lg border border-border-soft bg-surface-sunken">
        {runs === null ? (
          <div className="flex h-full items-center justify-center">
            <p className="font-sans text-xs tracking-[0.08em] text-text-dim">
              Loading runs…
            </p>
          </div>
        ) : list.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-center gap-3 py-10">
            <div className="grid h-12 w-12 place-items-center rounded-md border border-border-soft bg-surface-overlay text-text">
              <Terminal className="h-5 w-5" strokeWidth={2.5} />
            </div>
            <div className="space-y-1">
              <p className="font-sans text-sm font-bold tracking-[0.08em] text-text">
                No recorded runs yet
              </p>
              <p className="max-w-md text-xs text-text-dim font-sans leading-relaxed">
                Every command an agent executes through{" "}
                <code className="font-mono text-text-muted">ministr_run</code>{" "}
                lands here — exit code, duration, and the full captured log.
                Turn on exec-only steering with{" "}
                <code className="font-mono text-text-muted">
                  ministr init --exec-only
                </code>
                .
              </p>
            </div>
          </div>
        ) : (
          <ul className="flex flex-col">
            {list.map((run) => (
              <RunRow
                key={run.run_id}
                run={run}
                now={clock}
                expanded={expanded === run.run_id}
                onToggle={() =>
                  setExpanded((cur) => (cur === run.run_id ? null : run.run_id))
                }
              />
            ))}
          </ul>
        )}
      </div>

      {/* Vitals footer */}
      {/* text-mono-mini, not text-xs: the app's xs computes to 10.5px,
          below the 11px legibility floor the scrutiny probe enforces. */}
      <footer className="flex items-center justify-between gap-3 border-t border-border bg-surface-overlay px-3 py-1 shrink-0 font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
        <span>
          {list.length} {list.length === 1 ? "RUN" : "RUNS"}
        </span>
        <span className="flex items-center gap-3">
          <span className="inline-flex items-center gap-1.5">
            <StatusDot tone="accent" pulse={isLive ? "live" : "off"} />
            {activeCount} ACTIVE
          </span>
          <span className="inline-flex items-center gap-1.5">
            <StatusDot tone="danger" pulse="off" />
            {failedCount} FAILED
          </span>
        </span>
      </footer>
    </div>
  );
}

// ─── RUN ROW ────────────────────────────────────────────────────────────────

function RunRow({
  run,
  now,
  expanded,
  onToggle,
}: {
  run: ExecRun;
  now: number;
  expanded: boolean;
  onToggle: () => void;
}) {
  const p = runPresentation(run);
  // Severity reads from the NON-text left stripe + status dot; every word
  // stays text-text / text-muted so contrast holds on the sunken surface.
  const stripeClass =
    p.tone === "danger"
      ? "border-l-danger"
      : p.tone === "warning"
        ? "border-l-warning"
        : p.live
          ? "border-l-accent"
          : "border-l-transparent";

  return (
    <li
      className={cn(
        "border-b border-border-soft last:border-b-0",
        p.live && "bg-accent/5",
      )}
    >
      <button
        onClick={onToggle}
        aria-expanded={expanded}
        className={cn(
          "flex w-full items-center gap-3 border-l-2 px-3 py-2 text-left cursor-pointer transition-colors duration-150 ease-out hover:bg-surface-overlay focus-visible:outline-2 focus-visible:outline-accent",
          stripeClass,
        )}
      >
        <span className="inline-flex w-24 shrink-0 items-center gap-1.5 font-mono text-mono-micro font-medium uppercase tracking-[0.06em] text-text">
          <StatusDot tone={p.tone} pulse={p.live ? "live" : "off"} />
          {p.label}
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-mono-mini text-text">
          {run.command}
        </span>
        <span className="shrink-0 font-mono text-mono-micro tabular-nums text-text-muted">
          {formatDuration(run, now)}
        </span>
        <span className="hidden sm:inline shrink-0 font-mono text-mono-micro tabular-nums text-text-dim">
          {formatBytes(run.bytes_total)}
        </span>
        {run.session_id && (
          <span className="hidden md:inline shrink-0 rounded-full border border-border-soft bg-surface px-2 py-0.5 font-mono text-mono-micro text-text-muted">
            {run.session_id}
          </span>
        )}
      </button>

      {expanded && (
        <div
          role="region"
          aria-label={`Captured log for ${run.command}`}
          className="border-t border-border-soft bg-surface px-3 py-2"
        >
          {run.status === "running" ? (
            <p className="font-sans text-xs text-text-dim leading-relaxed">
              Running — output is captured by the run engine and appears
              here when the command finishes.
            </p>
          ) : run.log.length === 0 ? (
            <p className="font-sans text-xs text-text-dim">
              The command produced no output.
            </p>
          ) : (
            <pre className="max-h-72 overflow-y-auto whitespace-pre-wrap break-all font-mono text-mono-mini leading-relaxed">
              {run.log.split("\n").map((line, i) => (
                <span
                  key={i}
                  className={cn(
                    "block border-l-2 pl-2",
                    lineLevel(line) === "error"
                      ? "border-l-danger text-text"
                      : lineLevel(line) === "warn"
                        ? "border-l-warning text-text"
                        : "border-l-transparent text-text-muted",
                  )}
                >
                  {line}
                </span>
              ))}
            </pre>
          )}
          {/* Values stay verbatim (paths + fingerprints are case-sensitive
              data) — only the eyebrow labels get the uppercase treatment. */}
          <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 font-mono text-mono-micro text-text-dim">
            <span>
              <span className="uppercase tracking-[0.06em]">cwd</span>{" "}
              <span className="text-text-muted">{run.cwd}</span>
            </span>
            <span>
              <span className="uppercase tracking-[0.06em]">env</span>{" "}
              <span className="text-text-muted">{run.env_fingerprint}</span>
            </span>
            {run.truncated && (
              <span className="uppercase tracking-[0.06em]">
                output guard trimmed the middle
              </span>
            )}
          </div>
        </div>
      )}
    </li>
  );
}
