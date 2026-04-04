import { cn } from "../../lib/utils";

const variants = {
  default: "bg-accent hover:bg-accent-hover text-white",
  ghost: "hover:bg-surface-overlay text-text-muted hover:text-text",
  danger: "hover:bg-danger/10 text-text-muted hover:text-danger border border-border hover:border-danger",
  outline: "border border-border hover:bg-surface-overlay text-text-muted hover:text-text",
} as const;

const sizes = {
  sm: "h-7 px-2.5 text-xs",
  default: "h-8 px-3 text-sm",
  lg: "h-9 px-4 text-sm",
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
        "inline-flex items-center justify-center rounded-md font-medium transition-colors cursor-pointer",
        "disabled:pointer-events-none disabled:opacity-50",
        variants[variant],
        sizes[size],
        className,
      )}
      {...props}
    />
  );
}
