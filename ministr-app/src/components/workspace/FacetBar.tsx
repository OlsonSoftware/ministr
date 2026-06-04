import { motion } from "motion/react";
import { spring } from "../../lib/motion";
import { cn } from "../../lib/utils";
import { FACETS } from "./facets";
import { useWorkspace } from "./WorkspaceContext";

/**
 * The facet switcher — the workspace's primary in-context navigation, rendered
 * as a segmented "deck control" that completes the command-deck top chrome with
 * the ScopeHeader above it. The four stable verbs (Ask·Explore·Activity·Tend)
 * apply to whatever object the spine has selected; switching a facet never
 * re-selects the object.
 *
 * The active tab is a lifted pill (a shared-layout slide) with the facet's icon
 * in accent and an accent under-bar; every tab carries a quiet, always-visible
 * keyboard-chord hint. Single-accent system: the active facet lights in THE
 * accent — there is no per-facet colour (that would break token cohesion).
 */
export function FacetBar() {
  const { facet, setFacet } = useWorkspace();
  return (
    <nav
      aria-label="Facets"
      className="flex h-12 shrink-0 items-center border-b border-border bg-surface-raised px-3"
    >
      {/* The segmented track — an inset instrument the tabs ride in. */}
      <div
        role="tablist"
        aria-label="Facets"
        className="inline-flex items-center gap-0.5 rounded-lg border border-border-soft bg-surface-sunken p-1"
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
              title={f.blurb}
              onClick={() => setFacet(f.id)}
              className={cn(
                "group relative flex h-7 items-center gap-2 rounded-md px-2.5 cursor-pointer",
                "transition-colors duration-150 ease-out",
                "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent focus-visible:z-20",
                isActive
                  ? "text-text"
                  : "text-text-dim hover:text-text hover:bg-surface-overlay",
              )}
            >
              {/* The lifted active pill — slides between tabs via shared layout,
                  with a soft shadow + a bottom accent bar for the glow. */}
              {isActive && (
                <motion.span
                  layoutId="facet-active"
                  transition={spring}
                  className="absolute inset-0 rounded-md border border-border bg-surface shadow-sm"
                >
                  <span className="absolute inset-x-1.5 bottom-0 h-0.5 rounded-full bg-accent" />
                </motion.span>
              )}
              <span className="relative z-10 flex items-center gap-2">
                <Icon
                  className={cn(
                    "h-4 w-4 shrink-0 transition-colors",
                    isActive ? "text-accent" : "text-text-dim group-hover:text-text",
                  )}
                  strokeWidth={2}
                />
                <span className="font-sans text-xs font-medium">{f.label}</span>
                <Chord chord={f.chord} active={isActive} />
              </span>
            </button>
          );
        })}
      </div>
    </nav>
  );
}

/** A quiet, always-visible keyboard-chord hint (e.g. `g a`). Rendered as a
 *  faint mono badge so the deck advertises its shortcuts the way a pro tool
 *  does, without competing with the label. Hidden from the a11y tree (the
 *  chord lives in the tab's `title`). */
function Chord({ chord, active }: { chord: string; active: boolean }) {
  return (
    <span
      aria-hidden
      className={cn(
        "ml-0.5 inline-flex items-center gap-0.5 font-mono text-mono-micro uppercase tracking-[0.08em]",
        active ? "text-text-muted" : "text-text-dim",
      )}
    >
      {chord.split(" ").map((k, i) => (
        <kbd
          key={i}
          className={cn(
            "grid h-3.5 min-w-3.5 place-items-center rounded border px-1 leading-none",
            active
              ? "border-border bg-surface-overlay"
              : "border-border-soft bg-surface",
          )}
        >
          {k}
        </kbd>
      ))}
    </span>
  );
}
