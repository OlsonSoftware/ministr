import type { ReactNode } from "react";
import type { LucideIcon } from "@/components/ui/icons";
import { cn } from "../../lib/utils";
import { H1 } from "./heading";

/**
 * FacetHeader — the shared title row every workspace FACET wears, so the
 * facets read as ONE workspace (aaa-views-cohesion-sweep). It is to facets
 * what `LensHeader` (ui/lens-frame) is to Explore's lenses: one grammar for
 * the identity row — an identity icon echoing the FacetBar tab, the `H1` screen
 * title, an optional inline `scope` chip (the facet's object scope), a muted
 * dot-separated `glance` stat line, and a right-aligned `actions` slot — plus
 * an optional `children` block (e.g. a vitals tile-grid) below it.
 *
 * Owns the standard facet padding so spacing is consistent across facets. A
 * facet that hosts the row inside its OWN padded layout (e.g. Ask's two-column
 * input surface) passes `bare` to drop the padding and avoid double-insetting.
 */
export interface FacetHeaderProps {
  /** The facet's screen title (sentence case) — rendered via the `H1` atom. */
  title: string;
  /** Optional identity icon, accent-toned, left of the title. */
  icon?: LucideIcon;
  /** Optional inline scope chip after the title (e.g. Ask's active corpus). */
  scope?: ReactNode;
  /** The muted "N · M" glance stat line under the title. */
  glance?: ReactNode;
  /** Right-aligned actions (buttons / aggregate stats). */
  actions?: ReactNode;
  /** Optional sub-row content under the title row (e.g. a vitals tile grid). */
  children?: ReactNode;
  /** Drop the built-in facet padding when the host owns its own layout padding. */
  bare?: boolean;
}

export function FacetHeader({
  title,
  icon: Icon,
  scope,
  glance,
  actions,
  children,
  bare = false,
}: FacetHeaderProps) {
  return (
    <header className={cn("shrink-0", !bare && "px-5 pt-5 pb-4")}>
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2 min-w-0">
            {Icon && (
              <Icon
                className="h-5 w-5 shrink-0 text-accent"
                strokeWidth={2}
                aria-hidden
              />
            )}
            <H1>{title}</H1>
            {scope != null && (
              <span className="font-mono text-xs uppercase tracking-[0.08em] text-text-dim truncate">
                {scope}
              </span>
            )}
          </div>
          {glance != null && (
            <p className="font-sans text-sm text-text-dim mt-1">{glance}</p>
          )}
        </div>
        {actions && (
          <div className="flex items-center gap-2 shrink-0">{actions}</div>
        )}
      </div>
      {children && <div className="mt-4">{children}</div>}
    </header>
  );
}
