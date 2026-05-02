import type React from "react";
import { type Tone, toneTextClass } from "../../lib/status";
import { cn } from "../../lib/utils";

type MetricVariant = "tile" | "inline" | "compact";

interface MetricTileProps {
  /** Optional leading icon. */
  icon?: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  label: string;
  value: React.ReactNode;
  /** Optional tone tint applied to the value (and icon). */
  tone?: Tone;
  /** Layout variant: `tile` (framed), `inline` (single row), `compact` (stacked). */
  variant?: MetricVariant;
  /** Fade the whole tile (e.g. value is undefined). Only for `compact`. */
  muted?: boolean;
  className?: string;
}

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
      <span className={cn("flex items-center gap-1 text-text-muted", className)}>
        {Icon && <Icon className="h-3 w-3 text-text-dim" strokeWidth={2.5} />}
        <span className={cn("tabular-nums font-mono font-semibold", tone && toneTextClass(tone))}>
          {value}
        </span>
        <span className="font-mono text-xs tracking-[0.05em] text-text-dim">
          {label}
        </span>
      </span>
    );
  }

  if (variant === "compact") {
    return (
      <div className={cn("flex flex-col", className)}>
        <div
          className={cn(
            "flex items-center gap-1 font-mono font-bold tabular-nums",
            muted ? "text-text-dim" : tone ? toneTextClass(tone) : "text-text",
          )}
        >
          {Icon && <Icon className="h-3 w-3" strokeWidth={2.5} />}
          <span>{value}</span>
        </div>
        <span className="text-[0.5625rem] font-mono tracking-[0.05em] text-text-dim mt-0.5">
          {label}
        </span>
      </div>
    );
  }

  // tile (default)
  return (
    <div
      className={cn(
        "flex items-center gap-2.5 border border-border-soft bg-surface px-2.5 py-2",
        className,
      )}
    >
      {Icon && (
        <div className="grid h-7 w-7 place-items-center border border-border-soft bg-surface-overlay text-text">
          <Icon className="h-3.5 w-3.5" strokeWidth={2.5} />
        </div>
      )}
      <div className="min-w-0 flex-1">
        <p className="text-xs font-mono font-semibold tracking-[0.05em] text-text-dim">
          {label}
        </p>
        <p
          className={cn(
            "text-sm font-mono font-bold tabular-nums truncate",
            tone ? toneTextClass(tone) : "text-text",
          )}
        >
          {value}
        </p>
      </div>
    </div>
  );
}
