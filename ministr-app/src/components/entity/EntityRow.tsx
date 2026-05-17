import { ChevronRight } from "lucide-react";
import { cn } from "../../lib/utils";

interface Props {
  tag?: string;
  name: string;
  subtitle?: string;
  meta?: string;
  onClick?: () => void;
  className?: string;
}

/**
 * Cockpit row primitive used inside EntityPanel sections. Hairline
 * separators; hover lifts the surface and slides the chevron.
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
        "group w-full text-left flex items-start gap-3 px-3.5 py-2.5",
        "border-b border-border-soft last:border-b-0",
        "transition-colors duration-150",
        interactive
          ? "cursor-pointer hover:bg-surface-overlay focus-visible:bg-surface-overlay"
          : "cursor-default",
        className,
      )}
    >
      {tag && (
        <span className="font-mono text-mono-mini font-medium uppercase tracking-[0.08em] text-text-dim w-16 shrink-0 mt-0.5">
          {tag}
        </span>
      )}
      <span className="min-w-0 flex-1">
        <span className="font-mono text-sm font-semibold text-text truncate block group-hover:text-accent transition-colors duration-150">
          {name}
        </span>
        {subtitle && (
          <span className="font-mono text-xs text-text-dim truncate block mt-0.5">
            {subtitle}
          </span>
        )}
      </span>
      {meta && (
        <span className="font-mono text-xs tabular-nums text-text-dim shrink-0 mt-0.5">
          {meta}
        </span>
      )}
      {interactive && (
        <ChevronRight
          className="h-3.5 w-3.5 shrink-0 mt-0.5 text-text-dim group-hover:text-accent group-hover:translate-x-0.5 transition-all duration-150"
          strokeWidth={2}
        />
      )}
    </button>
  );
}
