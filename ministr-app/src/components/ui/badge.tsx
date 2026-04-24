import { cn } from "../../lib/utils";

const variants = {
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
} as const;

interface BadgeProps {
  variant?: keyof typeof variants;
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
            variant === "default" && "bg-accent",
            variant === "success" && "bg-success",
            variant === "warning" && "bg-warning",
            variant === "danger" && "bg-danger",
            variant === "muted" && "bg-text-dim",
          )}
        />
      )}
      {children}
    </span>
  );
}
