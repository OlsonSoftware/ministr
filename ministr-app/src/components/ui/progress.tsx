import { type Tone, toneBgClass } from "../../lib/status";
import { cn } from "../../lib/utils";

interface ProgressProps {
  value: number; // 0-100
  className?: string;
  /** Color of the fill. Defaults to `accent`. */
  tone?: Tone;
  /** Legacy prop kept for compatibility — brutalist progress bars don't glow. */
  glow?: boolean;
}

export function Progress({
  value,
  className,
  tone = "accent",
}: ProgressProps) {
  const pct = Math.min(100, Math.max(0, value));
  return (
    <div
      className={cn(
        "relative h-2 w-full overflow-hidden border border-border-soft bg-surface-overlay",
        className,
      )}
    >
      <div
        className={cn("h-full transition-none", toneBgClass(tone))}
        style={{ width: `${pct}%` }}
      />
    </div>
  );
}
