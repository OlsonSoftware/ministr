import type { ReactNode } from "react";
import type { LucideIcon } from "lucide-react";
import { H1 } from "./heading";

/**
 * FacetHeader — the shared title row every workspace FACET wears, so the
 * facets read as ONE workspace (aaa-views-cohesion-sweep). It is to facets
 * what `LensHeader` (ui/lens-frame) is to Explore's lenses: one grammar for
 * the identity row — an optional identity icon, the `H1` screen title, a muted
 * dot-separated `glance` stat line, and a right-aligned `actions` slot — plus
 * an optional `children` block (e.g. a vitals tile-grid) below it.
 *
 * Owns the standard facet padding so spacing is consistent across facets.
 */
export interface FacetHeaderProps {
  /** The facet's screen title (sentence case) — rendered via the `H1` atom. */
  title: string;
  /** Optional identity icon, accent-toned, left of the title. */
  icon?: LucideIcon;
  /** The muted "N · M" glance stat line under the title. */
  glance?: ReactNode;
  /** Right-aligned actions (buttons / aggregate stats). */
  actions?: ReactNode;
  /** Optional sub-row content under the title row (e.g. a vitals tile grid). */
  children?: ReactNode;
}

export function FacetHeader({
  title,
  icon: Icon,
  glance,
  actions,
  children,
}: FacetHeaderProps) {
  return (
    <header className="shrink-0 px-5 pt-5 pb-4">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            {Icon && (
              <Icon
                className="h-5 w-5 shrink-0 text-accent"
                strokeWidth={2}
                aria-hidden
              />
            )}
            <H1>{title}</H1>
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
