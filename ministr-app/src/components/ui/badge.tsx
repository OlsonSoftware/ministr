import { cn } from "../../lib/utils";
import { type BadgeVariant, toneBgClass } from "../../lib/status";

const variants: Record<BadgeVariant, string> = {
  default:
    "bg-[var(--color-accent-soft)] text-accent border-[var(--color-accent-ring)]",
  success:
    "bg-success/10 text-success border-success/25",
  warning:
    "bg-warning/10 text-warning border-warning/30",
  danger:
    "bg-danger/10 text-danger border-danger/30",
  muted:
    "bg-surface-overlay text-text-muted border-border",
};

const dotTones: Record<BadgeVariant, "accent" | "success" | "warning" | "danger" | "muted"> = {
  default: "accent",
  success: "success",
  warning: "warning",
  danger: "danger",
  muted: "muted",
};

interface BadgeProps {
  variant?: BadgeVariant;
  children: React.ReactNode;
  className?: string;
  /** Tiny pulsing dot for live/active states. */
  dot?: boolean;
}

export function Badge({
  variant = "default",
  children,
  className,
  dot = false,
}: BadgeProps) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] font-medium leading-tight",
        variants[variant],
        className,
      )}
    >
      {dot && (
        <span
          className={cn(
            "ministr-pulse h-1.5 w-1.5 rounded-full",
            toneBgClass(dotTones[variant]),
          )}
        />
      )}
      {children}
    </span>
  );
}
