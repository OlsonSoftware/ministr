import { type Tone, toneBgClass } from "../../lib/status";
import { cn } from "../../lib/utils";

interface ProgressProps {
  value: number; // 0-100
  className?: string;
  /** Color of the fill. Defaults to `accent`. */
  tone?: Tone;
  /** Add a soft accent glow to the fill (for live/active progress). */
  glow?: boolean;
}

/** Cockpit progress bar — rounded track, animated fill width. */
export function Progress({
  value,
  className,
  tone = "accent",
  glow = false,
}: ProgressProps) {
  const pct = Math.min(100, Math.max(0, value));
  return (
    <div
      role="progressbar"
      aria-valuenow={Math.round(pct)}
      aria-valuemin={0}
      aria-valuemax={100}
      className={cn(
        "relative h-1.5 w-full overflow-hidden rounded-full bg-surface-overlay",
        className,
      )}
    >
      <div
        className={cn(
          "h-full rounded-full transition-[width] duration-300 ease-out",
          toneBgClass(tone),
          glow && "shadow-[var(--glow-soft)]",
        )}
        style={{ width: `${pct}%` }}
      />
    </div>
  );
}
