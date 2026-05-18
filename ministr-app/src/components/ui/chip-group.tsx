import type { ReactNode } from "react";
import { cn } from "../../lib/utils";

interface ChipProps {
  label: ReactNode;
  /** Optional right-aligned count rendered next to the label. */
  count?: number;
  active?: boolean;
  onClick?: () => void;
  /** When set, renders an `<a>` instead of `<button>` (uncommon — used for
   *  count-only display chips that aren't toggles). */
  asStatic?: boolean;
  className?: string;
}

/**
 * Single chip with a label + optional count. Promoted from the local
 * `FilterChip` that lived inside `ActivityTimeline.tsx` so it can be
 * reused by the new code-touched section and any other filter UI.
 */
export function Chip({
  label,
  count,
  active = false,
  onClick,
  asStatic = false,
  className,
}: ChipProps) {
  const body = (
    <>
      <span>{label}</span>
      {count !== undefined && (
        <span
          className={cn(
            "tabular-nums",
            active ? "text-accent/70" : "text-text-dim/60",
          )}
        >
          {count}
        </span>
      )}
    </>
  );

  // Pill styling — quiet by default, gently lit when active. Smaller type
  // and tighter padding than the previous design so the filter bar feels
  // like metadata, not chrome.
  const classes = cn(
    "inline-flex items-center gap-1 whitespace-nowrap rounded-md border px-1.5 py-px font-mono text-[10px] uppercase tracking-[0.06em] leading-[18px] transition-colors duration-150",
    active
      ? "border-accent/40 bg-accent/10 text-accent"
      : "border-border-soft/60 text-text-dim hover:border-border-soft hover:text-text-muted",
    onClick && "cursor-pointer",
    className,
  );

  if (asStatic) {
    return <span className={classes}>{body}</span>;
  }
  return (
    <button type="button" onClick={onClick} className={classes}>
      {body}
    </button>
  );
}

/**
 * Wrapping container for a row of chips — wraps cleanly on narrow
 * widths and provides consistent spacing.
 */
export function ChipGroup({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex flex-wrap items-center gap-1.5", className)}>
      {children}
    </div>
  );
}
