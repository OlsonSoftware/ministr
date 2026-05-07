import { cn } from "../../lib/utils";
import { type BadgeVariant, toneBgClass } from "../../lib/status";

const variants: Record<BadgeVariant, string> = {
  default: "bg-surface text-text border-border-soft",
  success: "bg-surface text-success border-success",
  warning: "bg-surface text-warning border-warning",
  danger: "bg-surface text-danger border-danger",
  muted: "bg-surface text-text-muted border-border-soft",
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
  /** Tiny static square dot to reinforce variant color. */
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
        "inline-flex items-center gap-1.5 border px-2 py-1 text-mono-mini font-mono font-semibold uppercase tracking-[0.05em] leading-tight rounded-sm",
        variants[variant],
        className,
      )}
    >
      {dot && (
        <span
          className={cn("h-1.5 w-1.5", toneBgClass(dotTones[variant]))}
        />
      )}
      {children}
    </span>
  );
}
