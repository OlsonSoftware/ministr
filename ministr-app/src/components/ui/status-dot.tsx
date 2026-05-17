import { cn } from "../../lib/utils";
import { type Tone, toneBgClass } from "../../lib/status";

interface StatusDotProps {
  tone?: Tone;
  /** `"live"` runs the soft accent-glow pulse; `"off"` is static. */
  pulse?: "live" | "off";
  /** Bigger dot for headers. */
  size?: "sm" | "md";
  className?: string;
}

/**
 * Cockpit status indicator — a round dot with an optional soft pulse
 * (designed glow ring, not the old hard-step blink).
 */
export function StatusDot({
  tone = "muted",
  pulse = "off",
  size = "sm",
  className,
}: StatusDotProps) {
  const dim = size === "md" ? "h-2.5 w-2.5" : "h-2 w-2";
  return (
    <span
      className={cn(
        "inline-block shrink-0 rounded-full",
        dim,
        toneBgClass(tone),
        pulse === "live" && "ministr-pulse",
        className,
      )}
      aria-hidden="true"
    />
  );
}
