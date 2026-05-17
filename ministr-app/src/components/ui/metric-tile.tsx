import type React from "react";
import { type Tone, toneTextClass } from "../../lib/status";
import { cn } from "../../lib/utils";

type MetricVariant = "tile" | "inline" | "compact" | "cell";

interface MetricTileProps {
  icon?: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  label: string;
  value: React.ReactNode;
  tone?: Tone;
  /** Layout variant: `tile` (framed), `inline` (row), `compact` (stacked),
   *  `cell` (borderless, for divide-* grids). */
  variant?: MetricVariant;
  /** Fade the whole tile (value undefined). Only for `compact`. */
  muted?: boolean;
  className?: string;
}

const LABEL = "font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim";

export function MetricTile({
  icon: Icon,
  label,
  value,
  tone,
  variant = "tile",
  muted = false,
  className,
}: MetricTileProps) {
  if (variant === "inline") {
    return (
      <span className={cn("flex items-center gap-1.5 text-text-muted", className)}>
        {Icon && <Icon className="h-3 w-3 text-text-dim" strokeWidth={2} />}
        <span
          className={cn(
            "tabular-nums font-mono font-semibold",
            tone && toneTextClass(tone),
          )}
        >
          {value}
        </span>
        <span className={LABEL}>{label}</span>
      </span>
    );
  }

  if (variant === "compact") {
    return (
      <div className={cn("flex flex-col", className)}>
        <div
          className={cn(
            "flex items-center gap-1.5 font-mono font-semibold tabular-nums",
            muted ? "text-text-dim" : tone ? toneTextClass(tone) : "text-text",
          )}
        >
          {Icon && <Icon className="h-3 w-3" strokeWidth={2} />}
          <span>{value}</span>
        </div>
        <span className={cn(LABEL, "mt-0.5")}>{label}</span>
      </div>
    );
  }

  if (variant === "cell") {
    if (Icon) {
      return (
        <div
          className={cn("flex items-center gap-3 px-3 py-2.5 min-w-0", className)}
        >
          <div className="grid h-8 w-8 place-items-center rounded-md border border-border bg-surface-overlay text-text-muted shrink-0">
            <Icon className="h-3.5 w-3.5" strokeWidth={2} />
          </div>
          <div className="min-w-0 flex-1">
            <p className={LABEL}>{label}</p>
            <p
              className={cn(
                "font-mono text-base font-semibold tabular-nums truncate",
                tone ? toneTextClass(tone) : "text-text",
              )}
            >
              {value}
            </p>
          </div>
        </div>
      );
    }
    return (
      <div className={cn("px-3 py-2", className)}>
        <p className={LABEL}>{label}</p>
        <p
          className={cn(
            "font-mono text-base font-semibold tabular-nums mt-0.5 truncate",
            tone ? toneTextClass(tone) : "text-text",
          )}
        >
          {value}
        </p>
      </div>
    );
  }

  // tile (default)
  return (
    <div
      className={cn(
        "flex items-center gap-3 rounded-lg border border-border bg-surface px-3 py-2.5",
        "transition-colors duration-200 ease-out hover:border-border-hover",
        className,
      )}
    >
      {Icon && (
        <div className="grid h-8 w-8 place-items-center rounded-md border border-border bg-surface-overlay text-text-muted shrink-0">
          <Icon className="h-3.5 w-3.5" strokeWidth={2} />
        </div>
      )}
      <div className="min-w-0 flex-1">
        <p className={LABEL}>{label}</p>
        <p
          className={cn(
            "text-sm font-mono font-semibold tabular-nums truncate",
            tone ? toneTextClass(tone) : "text-text",
          )}
        >
          {value}
        </p>
      </div>
    </div>
  );
}
