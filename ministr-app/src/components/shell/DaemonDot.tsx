import { useEffect, useRef, useState } from "react";
import type { DaemonStatus } from "../../lib/types";
import { StatusDot } from "../ui/status-dot";
import { cn } from "../../lib/utils";
import { useToast } from "./ToastTray";

interface Props {
  status: DaemonStatus | null;
  error: string | null;
  onOpenLogs?: () => void;
}

function formatUptime(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

/**
 * Status dot in the TopBar. Click opens a popover with daemon vitals
 * (version, uptime, memory, log path) and an "open log" action.
 *
 * Tone:
 *  - error → danger
 *  - any indexing corpus → warning
 *  - otherwise → success
 */
export function DaemonDot({ status, error, onOpenLogs }: Props) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const { toast } = useToast();

  useEffect(() => {
    if (!open) return;
    function onClick(e: MouseEvent) {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    window.addEventListener("mousedown", onClick);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onClick);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const indexing = status
    ? status.corpora.filter((c) => c.status.state === "indexing").length
    : 0;
  const tone =
    error || !status
      ? ("danger" as const)
      : indexing > 0
        ? ("warning" as const)
        : ("success" as const);

  // Ambient status: a legible word woven into the chrome — not a dot you must
  // hover to read (aaa-chrome). Click still opens the vitals popover.
  const label =
    tone === "danger"
      ? "Offline"
      : tone === "warning"
        ? `Indexing ${indexing}`
        : "Ready";

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        title={
          tone === "danger"
            ? "Daemon disconnected"
            : tone === "warning"
              ? `Indexing ${indexing} project${indexing === 1 ? "" : "s"}`
              : "Daemon connected"
        }
        aria-label={`Daemon ${label}`}
        className={cn(
          "inline-flex items-center gap-1.5 h-8 pl-2 pr-2.5 rounded-md cursor-pointer shrink-0",
          "border border-border bg-surface hover:bg-surface-overlay hover:border-border-hover",
          "transition-colors duration-150",
          "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
        )}
      >
        <StatusDot tone={tone} pulse={tone === "warning" ? "live" : "off"} size="md" />
        <span
          className={cn(
            "font-mono text-mono-mini uppercase tracking-[0.08em]",
            tone === "danger" ? "text-danger" : "text-text-dim",
          )}
        >
          {label}
        </span>
      </button>

      {open && (
        <div className="absolute top-full right-0 mt-2 z-50 w-[300px] overflow-hidden rounded-lg border border-border bg-surface shadow-lg">
          <div className="border-b border-border bg-surface-overlay px-3 py-2">
            <span className="font-sans text-sm font-semibold text-text">
              Daemon
            </span>
          </div>
          <div className="p-3 space-y-1">
            <Row
              label="STATUS"
              value={
                tone === "danger"
                  ? "DISCONNECTED"
                  : tone === "warning"
                    ? "INDEXING"
                    : "CONNECTED"
              }
            />
            {status && (
              <>
                <Row label="VERSION" value={`v${status.version}`} />
                <Row label="UPTIME" value={formatUptime(status.uptime_secs)} />
                <Row label="MEMORY" value={`${status.memory_mb.toFixed(0)} MB`} />
                <Row label="MODEL" value={status.model} truncate />
                <Row label="DIM" value={`${status.model_dimension}d`} />
              </>
            )}
            {error && (
              <div className="rounded-md border border-danger/40 bg-surface-overlay px-2 py-1.5 mt-2 font-mono text-xs text-danger break-words">
                {error}
              </div>
            )}
          </div>
          {status?.log_path && onOpenLogs && (
            <button
              onClick={() => {
                // The toast is owned by the App-level onOpenLogs callback
                // now, since only it knows whether the host opener
                // actually succeeded. DaemonDot just dispatches and
                // closes the popover.
                onOpenLogs();
                setOpen(false);
              }}
              className="w-full border-t border-border bg-surface text-text-muted hover:text-text hover:bg-surface-overlay cursor-pointer transition-colors duration-150 px-3 py-2 font-sans text-sm font-medium text-left focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-accent"
            >
              Open log file →
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function Row({
  label,
  value,
  truncate,
}: {
  label: string;
  value: string;
  truncate?: boolean;
}) {
  return (
    <div className="flex items-center justify-between gap-2">
      <span className="font-mono text-xs tracking-[0.08em] text-text-dim shrink-0">
        {label}
      </span>
      <span
        className={cn(
          "font-mono text-mono-mini tabular-nums text-text",
          truncate && "truncate",
        )}
      >
        {value}
      </span>
    </div>
  );
}
