import { cn } from "../../lib/utils";

type Tone = "success" | "warning" | "danger" | "accent" | "muted";

const colors: Record<Tone, string> = {
  success: "bg-success",
  warning: "bg-warning",
  danger: "bg-danger",
  accent: "bg-accent",
  muted: "bg-text-dim",
};

interface StatusDotProps {
  tone?: Tone;
  /** Pulse animation on/off. */
  pulse?: boolean;
  /** Bigger dot for headers. */
  size?: "sm" | "md";
  className?: string;
}

export function StatusDot({
  tone = "muted",
  pulse = false,
  size = "sm",
  className,
}: StatusDotProps) {
  const dim = size === "md" ? "h-2 w-2" : "h-1.5 w-1.5";
  return (
    <span className={cn("relative inline-flex", dim, className)}>
      {pulse && (
        <span
          className={cn(
            "absolute inset-0 rounded-full opacity-70 animate-ping",
            colors[tone],
          )}
        />
      )}
      <span
        className={cn(
          "relative inline-flex rounded-full",
          dim,
          colors[tone],
        )}
      />
    </span>
  );
}
