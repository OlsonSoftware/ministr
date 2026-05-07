import { cn } from "../../lib/utils";

const variants = {
  // `default` is the page's primary CTA — accent fill, hairline border, the
  // signature shadow dance kept but quieter. Should appear at most once per
  // view (Run, Save, Apply, etc.). All other actions go to `outline` / `ghost`.
  default:
    "bg-accent text-[var(--color-accent-fg-on)] border border-border " +
    "shadow-xs " +
    "hover:bg-accent-hover hover:shadow-sm " +
    "active:shadow-none",
  ghost:
    "border border-transparent text-text-muted hover:text-text hover:bg-surface-overlay",
  danger:
    "border border-border-soft bg-surface text-text hover:bg-danger hover:text-white hover:border-danger",
  // `outline` is the everyday button — hairline 1px border, no shadow,
  // just a faint surface lift on hover.
  outline:
    "border border-border-soft bg-surface text-text " +
    "hover:bg-surface-overlay hover:border-border",
  subtle:
    "border border-border-soft bg-surface-overlay text-text hover:bg-surface",
} as const;

const sizes = {
  sm: "h-7 px-2.5 text-xs gap-1.5",
  default: "h-9 px-3 text-sm gap-2",
  lg: "h-11 px-4 text-sm gap-2",
  icon: "h-9 w-9",
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
  style,
  ...props
}: ButtonProps) {
  return (
    <button
      className={cn(
        // Base: sans, sentence-case, semibold. Caps/tracking are no longer
        // forced — they only hurt legibility on multi-word labels. Callers
        // can opt in via `className="uppercase tracking-[0.05em]"` if needed.
        "inline-flex items-center justify-center font-sans font-semibold cursor-pointer transition-none rounded-sm",
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
}
