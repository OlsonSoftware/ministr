import { cn } from "../../lib/utils";
import { type Tone, toneBgClass } from "../../lib/status";

interface StatusDotProps {
  tone?: Tone;
  /** Pulse animation. `"live"` runs the ping; `"off"` is static. */
  pulse?: "live" | "off";
  /** Bigger dot for headers. */
  size?: "sm" | "md";
  className?: string;
}

export function StatusDot({
  tone = "muted",
  pulse = "off",
  size = "sm",
  className,
}: StatusDotProps) {
  const dim = size === "md" ? "h-2 w-2" : "h-1.5 w-1.5";
  const bg = toneBgClass(tone);
  return (
    <span className={cn("relative inline-flex", dim, className)}>
      {pulse === "live" && (
        <span
          className={cn(
            "absolute inset-0 rounded-full opacity-70 animate-ping",
            bg,
          )}
        />
      )}
      <span className={cn("relative inline-flex rounded-full", dim, bg)} />
    </span>
  );
}
