import { motion } from "motion/react";
import { spring } from "../../lib/motion";
import { cn } from "../../lib/utils";
import { FACETS } from "./facets";
import { useWorkspace } from "./WorkspaceContext";

/**
 * The facet switcher — the workspace's primary in-context navigation. The four
 * stable verbs (Ask·Explore·Activity·Tend) are applied to whatever object the
 * spine has selected; switching a facet never re-selects the object.
 */
export function FacetBar() {
  const { facet, setFacet } = useWorkspace();
  return (
    <nav
      aria-label="Facets"
      role="tablist"
      className="flex items-center gap-1 border-b border-border bg-surface px-2 h-11 shrink-0"
    >
      {FACETS.map((f) => {
        const Icon = f.icon;
        const isActive = f.id === facet;
        return (
          <button
            key={f.id}
            type="button"
            role="tab"
            aria-selected={isActive}
            title={`${f.label} · ${f.chord}`}
            onClick={() => setFacet(f.id)}
            className={cn(
              "group relative flex items-center gap-2 h-8 px-3 rounded-md cursor-pointer",
              "transition-colors duration-150",
              "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent focus-visible:z-20",
              isActive
                ? "text-text"
                : "text-text-dim hover:text-text hover:bg-surface-overlay",
            )}
          >
            {isActive && (
              <motion.span
                layoutId="facet-active"
                transition={spring}
                className="absolute inset-0 rounded-md border border-border bg-surface-overlay"
              />
            )}
            <span className="relative z-10 flex items-center gap-2">
              <Icon
                className={cn("h-4 w-4", isActive && "text-accent")}
                strokeWidth={2}
              />
              <span className="font-sans text-xs font-medium">{f.label}</span>
            </span>
          </button>
        );
      })}
    </nav>
  );
}
