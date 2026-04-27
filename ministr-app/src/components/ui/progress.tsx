import { type Tone, toneBgClass } from "../../lib/status";
import { cn } from "../../lib/utils";

interface ProgressProps {
  value: number; // 0-100
  className?: string;
  /** Color of the fill. Defaults to `accent`. Pass `warning`/`danger`
   *  for threshold-state bars; only those tones get the glow. */
  tone?: Tone;
  /** Force a glow even on the default tone. Reserve for moments where
   *  you actually want to draw the eye (e.g. a critical-pressure
   *  indicator). The bar is solid otherwise so idle progress doesn't
   *  scream for attention. */
  glow?: boolean;
}

export function Progress({
  value,
  className,
  tone = "accent",
  glow = false,
}: ProgressProps) {
  const pct = Math.min(100, Math.max(0, value));
  const showGlow = glow || tone === "warning" || tone === "danger";
  return (
    <div
      className={cn(
        "relative h-1.5 w-full overflow-hidden rounded-full bg-surface-overlay",
        className,
      )}
    >
      <div
        className={cn(
          "h-full rounded-full transition-all duration-300 ease-out",
          toneBgClass(tone),
          showGlow && "shadow-[0_0_8px_var(--color-accent-ring)]",
        )}
        style={{ width: `${pct}%` }}
      />
    </div>
  );
}
