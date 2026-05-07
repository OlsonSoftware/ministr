import { cn } from "../../lib/utils";

export type AskPhaseName =
  | "idle"
  | "analyzing"
  | "retrieving"
  | "reranking"
  | "synthesizing"
  | "verifying"
  | "done"
  | "error";

interface PhaseStripProps {
  phase: AskPhaseName;
  cached: boolean;
  /** Whether the verification step ran (always true for non-cached completions). */
  verified: boolean;
}

/**
 * Horizontal phase strip — replaces the old vertical PhaseRail.
 *
 * Each phase is a fixed-width block, and the active phase fills with the
 * accent-live gradient (which pulses to communicate "data is moving").
 * Completed phases are solid accent. Pending phases stay surface-overlay.
 *
 * On cache hits the whole strip skips to "done" with a small CACHED chip
 * since none of the actual stages ran.
 */
export function PhaseStrip({ phase, cached, verified }: PhaseStripProps) {
  if (cached && phase === "done") {
    return (
      <div className="flex items-center gap-2 border-2 border-accent bg-surface-overlay px-3 py-1.5">
        <span className="font-mono text-mono-mini font-semibold uppercase tracking-[0.05em] text-accent">
          Cache hit
        </span>
        <span className="flex-1 h-1 bg-accent" />
        <span className="font-mono text-mono-mini text-text-dim">
          ~10ms
        </span>
      </div>
    );
  }

  const phases: { key: AskPhaseName; label: string }[] = [
    { key: "analyzing", label: "Analyze" },
    { key: "retrieving", label: "Retrieve" },
    { key: "reranking", label: "Rerank" },
    { key: "synthesizing", label: "Synthesize" },
    { key: "verifying", label: "Verify" },
  ];

  const order = phases.map((p) => p.key);
  const activeIdx = order.indexOf(phase);

  return (
    <div
      className="flex items-stretch gap-[2px] border-2 border-border bg-surface"
      role="progressbar"
      aria-label={`Ask pipeline phase: ${phase}`}
    >
      {phases.map((p, i) => {
        // Pre-active = done, equal to active = active, after = pending.
        // Done state collapses everything to "done".
        const completed =
          phase === "done"
            ? true
            : phase === "error"
              ? false
              : i < activeIdx;
        const active =
          phase !== "done" && phase !== "error" && i === activeIdx;
        const isVerifyAndSkipped =
          p.key === "verifying" && phase === "done" && !verified;

        return (
          <div
            key={p.key}
            className={cn(
              "flex-1 flex items-center justify-center px-2 py-1.5 min-w-0 relative",
              completed && !isVerifyAndSkipped && "bg-accent text-accent-fg-on",
              active && "bg-accent-live text-accent-fg-on",
              !completed && !active && "bg-surface text-text-dim",
              isVerifyAndSkipped && "bg-surface text-text-dim",
            )}
          >
            <span className="font-mono text-mono-mini font-semibold uppercase tracking-[0.05em] truncate">
              {p.label}
            </span>
          </div>
        );
      })}
    </div>
  );
}
