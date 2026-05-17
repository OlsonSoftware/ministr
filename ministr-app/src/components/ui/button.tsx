import { forwardRef } from "react";
import { cn } from "../../lib/utils";

const variants = {
  // Primary CTA — accent fill + soft lift + live glow on hover. At most
  // once per view (Run, Save, Apply). Everything else is outline/ghost.
  default:
    "bg-accent text-[var(--color-accent-fg-on)] shadow-sm " +
    "hover:bg-accent-hover hover:shadow-[var(--glow-soft)] " +
    "active:scale-[0.98]",
  ghost:
    "text-text-muted hover:text-text hover:bg-surface-overlay active:scale-[0.98]",
  danger:
    "border border-border bg-surface text-text " +
    "hover:bg-danger hover:text-white hover:border-danger active:scale-[0.98]",
  // Everyday button — hairline border, faint surface lift.
  outline:
    "border border-border bg-surface text-text " +
    "hover:bg-surface-overlay hover:border-border-hover active:scale-[0.98]",
  subtle:
    "border border-border-soft bg-surface-overlay text-text " +
    "hover:bg-surface active:scale-[0.98]",
} as const;

const sizes = {
  sm: "h-7 px-2.5 text-xs gap-1.5",
  default: "h-9 px-3.5 text-sm gap-2",
  lg: "h-11 px-5 text-sm gap-2",
  icon: "h-9 w-9",
  "icon-sm": "h-7 w-7",
} as const;

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: keyof typeof variants;
  size?: keyof typeof sizes;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "default", size = "default", className, style, ...props },
  ref,
) {
  return (
    <button
      ref={ref}
      className={cn(
        "inline-flex items-center justify-center font-sans font-medium cursor-pointer rounded-md",
        "transition-[background-color,box-shadow,border-color,transform] duration-150 ease-out",
        "disabled:pointer-events-none disabled:opacity-50",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
        variants[variant],
        sizes[size],
        className,
      )}
      style={style}
      {...props}
    />
  );
});
