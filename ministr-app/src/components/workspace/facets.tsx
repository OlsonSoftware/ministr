import { Activity, Compass, MessageSquare, Sprout } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { FacetId } from "./WorkspaceContext";

/**
 * Facet registry — the stable verbs the workspace applies to the spine object.
 *
 * AAA-VISION.md maps the six old destinations onto four facets:
 *   Ask      ← the threaded Ask conversation
 *   Explore  ← the index (symbol graph, bridges, code browser)
 *   Activity ← the mission-control board (sessions + indexing + deliveries)
 *   Tend     ← per-project care (health, config, paths, reindex, sharing)
 *
 * This chunk (workspace-shell) ships the metadata + a scoped placeholder
 * renderer; the next chunk (workspace-facets) swaps in the shipped surfaces.
 */
export interface FacetMeta {
  id: FacetId;
  label: string;
  icon: LucideIcon;
  /** One line on what the facet does to the spine object. */
  blurb: string;
  /** Keyboard chord hint (g-prefix), shown in tooltips / the palette. */
  chord: string;
}

export const FACETS: readonly FacetMeta[] = [
  {
    id: "ask",
    label: "Ask",
    icon: MessageSquare,
    blurb: "Converse with the project — threaded questions, cited answers.",
    chord: "g a",
  },
  {
    id: "explore",
    label: "Explore",
    icon: Compass,
    blurb: "Browse the index — symbols, bridges, and source.",
    chord: "g e",
  },
  {
    id: "activity",
    label: "Activity",
    icon: Activity,
    blurb: "Live agents, indexing, and recent deliveries.",
    chord: "g s",
  },
  {
    id: "tend",
    label: "Tend",
    icon: Sprout,
    blurb: "Care for the project — health, config, paths, sharing.",
    chord: "g t",
  },
];

export const FACET_BY_ID: Record<FacetId, FacetMeta> = Object.fromEntries(
  FACETS.map((f) => [f.id, f]),
) as Record<FacetId, FacetMeta>;
