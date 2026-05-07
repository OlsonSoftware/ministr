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

  const tone =
    error || !status
      ? ("danger" as const)
      : status.corpora.some((c) => c.status.state === "indexing")
        ? ("warning" as const)
        : ("success" as const);

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        title={
          tone === "danger"
            ? "Daemon disconnected"
            : tone === "warning"
              ? "Indexing"
              : "Daemon connected"
        }
        className={cn(
          "grid place-items-center h-5 w-5 cursor-pointer transition-none",
        )}
      >
        <StatusDot tone={tone} pulse={tone === "warning" ? "live" : "off"} size="md" />
      </button>

      {open && (
        <div className="absolute top-full left-0 mt-1 z-50 w-[300px] border border-border-soft bg-surface shadow-md">
          <div className="border-b border-border-soft bg-surface-overlay px-3 py-2">
            <span className="font-serif text-base font-bold text-text">
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
              <div className="border-l-2 border-danger bg-surface-overlay px-2 py-1 mt-2 font-mono text-xs text-danger break-words">
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
              className="w-full border-t border-border-soft bg-surface text-text-muted hover:text-text hover:bg-surface-overlay cursor-pointer transition-none px-3 py-2 font-sans text-sm font-medium text-left"
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
      <span className="font-mono text-xs tracking-[0.05em] text-text-dim shrink-0">
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
