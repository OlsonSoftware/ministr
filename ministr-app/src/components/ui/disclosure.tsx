import type { ReactNode } from "react";
import { ChevronRight } from "lucide-react";
import { cn } from "../../lib/utils";

interface Props {
  title: string;
  /** Right-edge inline summary shown next to the title in the header. */
  meta?: ReactNode;
  /** Optional `§N` chapter index, mirroring `EntitySection`. */
  chapter?: number;
  defaultOpen?: boolean;
  className?: string;
  children: ReactNode;
}

/**
 * Collapsible card built on the native `<details>` element so it is
 * keyboard-accessible and screen-readable for free. Visual contract
 * mirrors `EntitySection`: rounded panel, hairline border, sans header
 * with optional `§N` gutter and a right-aligned `meta` slot. The chevron
 * rotates open via the `[&[open]]:rotate-90` selector — no JS state.
 */
export function Disclosure({
  title,
  meta,
  chapter,
  defaultOpen = false,
  className,
  children,
}: Props) {
  return (
    <details
      className={cn(
        "group overflow-hidden rounded-lg border border-border bg-surface",
        className,
      )}
      open={defaultOpen}
    >
      <summary
        className={cn(
          "flex cursor-pointer list-none items-baseline gap-3 border-b border-transparent bg-surface-overlay px-3.5 py-2.5",
          "group-open:border-border",
          "[&::-webkit-details-marker]:hidden",
        )}
      >
        <ChevronRight
          className="h-3.5 w-3.5 shrink-0 text-text-dim transition-transform duration-150 ease-out group-open:rotate-90"
          strokeWidth={2}
          aria-hidden="true"
        />
        {chapter !== undefined && (
          <span className="font-mono text-xs font-medium text-accent tabular-nums shrink-0">
            §{chapter}
          </span>
        )}
        <h3 className="font-sans text-base font-semibold tracking-[-0.005em] text-text leading-snug flex-1 min-w-0">
          {title}
        </h3>
        {meta !== undefined && (
          <span className="font-mono text-xs tabular-nums text-text-dim shrink-0">
            {meta}
          </span>
        )}
      </summary>
      <div>{children}</div>
    </details>
  );
}
