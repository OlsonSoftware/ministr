import type React from "react";
import { type Tone, toneTextClass } from "../../lib/status";
import { cn } from "../../lib/utils";

type MetricVariant = "tile" | "inline" | "compact";

interface MetricTileProps {
  /** Optional leading icon. Sized to match the variant. */
  icon?: React.ComponentType<{ className?: string }>;
  label: string;
  value: React.ReactNode;
  /** Optional tone tint applied to the value (and icon). When
   *  omitted the value uses default text color. */
  tone?: Tone;
  /** Layout variant.
   *  - `tile`: framed box, label-above, value-below (Project detail).
   *  - `inline`: single-row icon + value + label, low contrast (Project list metadata).
   *  - `compact`: stacked icon+value top, tiny label below (turn-block metric grid). */
  variant?: MetricVariant;
  /** Fade the whole tile (e.g. value is undefined). Only meaningful for `compact`. */
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
        {Icon && <Icon className="h-3 w-3 text-text-dim" />}
        <span className={cn("tabular-nums font-medium", tone && toneTextClass(tone))}>
          {value}
        </span>
        <span className="text-text-dim">{label}</span>
      </span>
    );
  }

  if (variant === "compact") {
    return (
      <div className={cn("flex flex-col", className)}>
        <div
          className={cn(
            "flex items-center gap-1 font-mono font-semibold tabular-nums",
            muted ? "text-text-dim" : tone ? toneTextClass(tone) : "text-text",
          )}
        >
          {Icon && <Icon className="h-3 w-3 opacity-70" />}
          <span>{value}</span>
        </div>
        <span className="text-[9px] uppercase tracking-wider text-text-dim mt-0.5">
          {label}
        </span>
      </div>
    );
  }

  // tile (default)
  return (
    <div
      className={cn(
        "flex items-center gap-2.5 rounded-lg border border-border/50 bg-surface/40 px-2.5 py-2",
        className,
      )}
    >
      {Icon && (
        <div className="grid h-7 w-7 place-items-center rounded-md bg-surface-overlay text-text-muted">
          <Icon className="h-3.5 w-3.5" />
        </div>
      )}
      <div className="min-w-0 flex-1">
        <p className="text-[10px] font-medium uppercase tracking-wider text-text-dim">
          {label}
        </p>
        <p
          className={cn(
            "text-sm font-semibold tabular-nums truncate",
            tone ? toneTextClass(tone) : "text-text",
          )}
        >
          {value}
        </p>
      </div>
    </div>
  );
}
