import { cn } from "../../lib/utils";

interface ProgressProps {
  value: number; // 0-100
  className?: string;
  /** Show the value as a dim track + accent fill with a subtle glow. */
  glow?: boolean;
}

export function Progress({ value, className, glow = false }: ProgressProps) {
  const pct = Math.min(100, Math.max(0, value));
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
          "bg-gradient-to-r from-accent to-[color-mix(in_srgb,var(--color-accent)_60%,#c4b5fd)]",
          glow &&
            "shadow-[0_0_8px_var(--color-accent-ring)]",
        )}
        style={{ width: `${pct}%` }}
      />
    </div>
  );
}
