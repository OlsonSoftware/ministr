import { cn } from "../../lib/utils";
import { type Tone, toneBgClass } from "../../lib/status";

interface StatusDotProps {
  tone?: Tone;
  /** `"live"` runs the hard-step blink; `"off"` is static. */
  pulse?: "live" | "off";
  /** Bigger dot for headers. */
  size?: "sm" | "md";
  className?: string;
}

/**
 * Brutalist status indicator — square (not circle), hard-step blink (not
 * a smooth pulse).
 */
export function StatusDot({
  tone = "muted",
  pulse = "off",
  size = "sm",
  className,
}: StatusDotProps) {
  const dim = size === "md" ? "h-2.5 w-2.5" : "h-2 w-2";
  const bg = toneBgClass(tone);
  return (
    <span
      className={cn(
        "inline-block shrink-0",
        dim,
        bg,
        pulse === "live" && "ministr-blink",
        className,
      )}
      aria-hidden="true"
    />
  );
}
