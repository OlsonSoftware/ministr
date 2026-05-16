import { cn } from "../../lib/utils";
import { type BadgeVariant, toneBgClass } from "../../lib/status";

const variants: Record<BadgeVariant, string> = {
  default: "bg-surface-overlay text-text-muted border-border",
  success: "bg-surface text-success border-success/40",
  warning: "bg-surface text-warning border-warning/40",
  danger: "bg-surface text-danger border-danger/40",
  muted: "bg-surface-overlay text-text-dim border-border-soft",
};

const dotTones: Record<
  BadgeVariant,
  "accent" | "success" | "warning" | "danger" | "muted"
> = {
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
  /** Small round dot reinforcing the variant color. */
  dot?: boolean;
}

/** Cockpit pill badge — fully rounded, hairline, mono caption. */
export function Badge({
  variant = "default",
  children,
  className,
  dot = false,
}: BadgeProps) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 border px-2 py-0.5 rounded-full",
        "text-mono-mini font-mono font-medium uppercase tracking-[0.06em] leading-tight",
        variants[variant],
        className,
      )}
    >
      {dot && (
        <span
          className={cn(
            "h-1.5 w-1.5 rounded-full",
            toneBgClass(dotTones[variant]),
          )}
        />
      )}
      {children}
    </span>
  );
}
