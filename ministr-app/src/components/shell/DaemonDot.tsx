import { useEffect, useRef, useState } from "react";
import { ArrowUpRight, ScrollText, Server } from "@/components/ui/icons";
import type { DaemonStatus } from "../../lib/types";
import { StatusDot } from "../ui/status-dot";
import { Badge } from "../ui/badge";
import { cn } from "../../lib/utils";
import { glassPanel } from "../../lib/ui-tokens";
import { toneTextClass } from "../../lib/status";

interface Props {
  status: DaemonStatus | null;
  error: string | null;
  onOpenLogs?: () => void;
}

/** Quiet tone tint for the popover identity medallion (no glow — the daemon
 *  popover is informational, not a "live" object). */
const TONE_RING: Record<"success" | "warning" | "danger", string> = {
  success: "border-success/40",
  warning: "border-warning/40",
  danger: "border-danger/40",
};

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

  // Short word for the popover header status pill (the vitals rows carry the
  // detail; the pill carries the one-word state).
  const statusWord =
    tone === "danger" ? "Offline" : tone === "warning" ? "Indexing" : "Ready";

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
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
        // Floating chrome on the Liquid-Glass tier (DESIGN.md §4): glassPanel
        // carries the blur + specular + reduced-transparency/forced-colors
        // solid fallbacks and owns the radius. overflow-hidden clips the
        // header/footer hairlines to the corner (safe — the specular is an
        // inset shadow, not a pseudo-element).
        <div
          role="region"
          aria-label="Daemon status"
          className={cn(
            glassPanel,
            "absolute top-full right-0 mt-2 z-50 w-[300px] overflow-hidden",
          )}
        >
          {/* Command-deck identity header: quiet tone medallion + name + the
              one-word status pill. Tone lives on the medallion + pill only. */}
          <div className="flex items-center gap-2.5 border-b border-border/70 px-3 py-2.5">
            <span
              aria-hidden
              className={cn(
                "grid h-8 w-8 shrink-0 place-items-center rounded-lg border bg-surface-overlay",
                TONE_RING[tone],
                toneTextClass(tone),
              )}
            >
              <Server className="h-4 w-4" strokeWidth={2} />
            </span>
            <div className="min-w-0 flex-1">
              <div className="font-sans text-sm font-semibold text-text leading-tight">
                Daemon
              </div>
              <div className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
                Code intelligence
              </div>
            </div>
            <Badge variant={tone} dot>
              {statusWord}
            </Badge>
          </div>

          {/* Vitals */}
          {status ? (
            <div className="px-3 py-2.5 space-y-1.5">
              <Row label="VERSION" value={`v${status.version}`} />
              <Row label="UPTIME" value={formatUptime(status.uptime_secs)} />
              <Row label="MEMORY" value={`${status.memory_mb.toFixed(0)} MB`} />
              <Row label="MODEL" value={status.model} truncate />
              <Row label="DIM" value={`${status.model_dimension}d`} />
            </div>
          ) : (
            !error && (
              <div className="px-3 py-3 font-sans text-xs text-text-dim">
                No daemon connection.
              </div>
            )
          )}

          {error && (
            <div className="mx-3 mb-3 mt-1 rounded-md border border-danger/40 bg-surface-overlay px-2 py-1.5 font-mono text-xs text-danger break-words">
              {error}
            </div>
          )}

          {status?.log_path && onOpenLogs && (
            <button
              onClick={() => {
                // The toast is owned by the App-level onOpenLogs callback now,
                // since only it knows whether the host opener actually
                // succeeded. DaemonDot just dispatches and closes the popover.
                onOpenLogs();
                setOpen(false);
              }}
              className={cn(
                "flex w-full items-center gap-2 border-t border-border/70 px-3 py-2.5 text-left",
                "font-sans text-sm font-medium text-text-muted hover:text-text hover:bg-surface-overlay/60",
                "cursor-pointer transition-colors duration-150",
                "focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-accent",
              )}
            >
              <ScrollText className="h-4 w-4 shrink-0" strokeWidth={2} />
              <span className="flex-1">Open log file</span>
              <ArrowUpRight className="h-4 w-4 shrink-0" strokeWidth={2} />
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
