import { ChevronRight } from "lucide-react";
import { cn } from "../../lib/utils";

interface Props {
  /** Short uppercase tag on the left — kind, role, or filter hint. */
  tag?: string;
  /** Bold mono name in the middle (rendered sentence-case as authored). */
  name: string;
  /** Optional secondary line under the name (mono dim). */
  subtitle?: string;
  /** Optional right-edge metadata (file:line, count, etc.). */
  meta?: string;
  onClick?: () => void;
  className?: string;
}

/**
 * Field-manual row primitive used inside EntityPanel sections. Hairline
 * separators between rows; no per-row border. Hover lifts the row with a
 * faint surface shade and underlines the name. Active focus is the only
 * place a row gets a bordered/shadowed treatment — that signature is left
 * to the parent (e.g. selected breadcrumb, focused search hit).
 */
export function EntityRow({
  tag,
  name,
  subtitle,
  meta,
  onClick,
  className,
}: Props) {
  const interactive = !!onClick;
  return (
    <button
      onClick={onClick}
      disabled={!interactive}
      className={cn(
        "group w-full text-left flex items-start gap-3 px-3 py-2 transition-none",
        "border-b border-border-soft last:border-b-0",
        interactive
          ? "cursor-pointer hover:bg-surface-overlay focus-visible:bg-surface-overlay"
          : "cursor-default",
        className,
      )}
    >
      {tag && (
        <span className="font-mono text-[0.6875rem] font-semibold uppercase tracking-[0.05em] text-text-dim w-16 shrink-0 mt-1">
          {tag}
        </span>
      )}
      <span className="min-w-0 flex-1">
        <span
          className={cn(
            "font-mono text-sm font-semibold text-text truncate block",
            interactive &&
              "border-b border-transparent group-hover:border-text",
          )}
        >
          {name}
        </span>
        {subtitle && (
          <span className="font-mono text-xs text-text-dim truncate block mt-0.5">
            {subtitle}
          </span>
        )}
      </span>
      {meta && (
        <span className="font-mono text-xs tabular-nums text-text-muted shrink-0 mt-1">
          {meta}
        </span>
      )}
      {interactive && (
        <ChevronRight
          className="h-3.5 w-3.5 shrink-0 mt-1 text-text-dim group-hover:text-text"
          strokeWidth={2}
        />
      )}
    </button>
  );
}
