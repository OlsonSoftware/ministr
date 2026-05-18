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
            active ? "opacity-80" : "text-text-dim",
          )}
        >
          {count}
        </span>
      )}
    </>
  );

  const classes = cn(
    "inline-flex items-center gap-1 whitespace-nowrap rounded-md border px-1.5 py-0.5 font-mono text-mono-mini uppercase tracking-[0.08em] transition-colors duration-150",
    active
      ? "border-accent bg-accent text-[var(--color-accent-fg-on)]"
      : "border-border-soft text-text-muted hover:border-border",
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
