import { cn } from "../../lib/utils";

const variants = {
  default:
    "bg-accent hover:bg-accent-hover text-[var(--color-accent-fg-on)] shadow-[inset_0_1px_0_rgb(255_255_255/0.15),0_1px_2px_rgb(0_0_0/0.25)] hover:shadow-[inset_0_1px_0_rgb(255_255_255/0.2),0_2px_6px_rgb(0_0_0/0.3)]",
  ghost:
    "hover:bg-surface-overlay text-text-muted hover:text-text",
  danger:
    "hover:bg-danger/10 text-text-muted hover:text-danger border border-border hover:border-danger/40",
  outline:
    "border border-border bg-surface-raised/40 hover:bg-surface-overlay hover:border-border-hover text-text-muted hover:text-text",
  subtle:
    "bg-surface-overlay/60 hover:bg-surface-overlay text-text hover:text-text border border-transparent hover:border-border",
} as const;

const sizes = {
  sm: "h-7 px-2.5 text-xs gap-1.5",
  default: "h-8 px-3 text-sm gap-2",
  lg: "h-10 px-4 text-sm gap-2",
  icon: "h-8 w-8",
  "icon-sm": "h-7 w-7",
} as const;

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: keyof typeof variants;
  size?: keyof typeof sizes;
}

export function Button({
  variant = "default",
  size = "default",
  className,
  ...props
}: ButtonProps) {
  return (
    <button
      className={cn(
        "inline-flex items-center justify-center rounded-md font-medium transition-all duration-150 cursor-pointer",
        "disabled:pointer-events-none disabled:opacity-50",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-accent-ring)]",
        variants[variant],
        sizes[size],
        className,
      )}
      {...props}
    />
  );
}
