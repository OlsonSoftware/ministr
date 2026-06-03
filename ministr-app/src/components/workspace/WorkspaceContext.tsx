import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import type { CorpusInfo } from "../../lib/types";

/**
 * The spine — the ONE selected object the whole workspace operates on.
 *
 * Project-as-spine (AAA-VISION.md): you are always looking at either a single
 * Project or the Fleet (the collection view of the *same* object, zoomed out).
 * Modeled as a discriminated union so a future dual-spine variant is purely
 * additive rather than a rewrite.
 */
export type Spine = { kind: "fleet" } | { kind: "project"; id: string };

/** The facets — stable verbs applied to the spine object. */
export type FacetId = "ask" | "explore" | "activity" | "tend";

export const FACET_IDS: readonly FacetId[] = [
  "ask",
  "explore",
  "activity",
  "tend",
] as const;

export interface WorkspaceContextValue {
  /** The selected object, chosen ONCE in the chrome spine picker. */
  spine: Spine;
  /** True when the spine is the Fleet (collection) view. */
  isFleet: boolean;
  /**
   * The active project id, or `null` when the spine is the Fleet. This is the
   * single source of truth for facet scoping — facets read THIS, never a
   * private `activeCorpusId`. That is the whole point of the integrated shell.
   */
  activeProjectId: string | null;
  /** The resolved active project, or `null` in Fleet / when none exists. */
  activeProject: CorpusInfo | null;
  /** Every known project (the Fleet). */
  corpora: CorpusInfo[];

  /** Select a single project as the spine. */
  selectProject: (id: string) => void;
  /** Zoom out to the Fleet (collection) view. */
  selectFleet: () => void;

  /** The active facet (stable across spine changes — the noun moves, not the verb). */
  facet: FacetId;
  setFacet: (facet: FacetId) => void;
}

const WorkspaceContext = createContext<WorkspaceContextValue | null>(null);

const SPINE_STORAGE_KEY = "ministr-spine";
const FACET_STORAGE_KEY = "ministr-facet";

function isFacetId(v: unknown): v is FacetId {
  return typeof v === "string" && (FACET_IDS as readonly string[]).includes(v);
}

function sameSpine(a: Spine, b: Spine): boolean {
  if (a.kind === "fleet" && b.kind === "fleet") return true;
  if (a.kind === "project" && b.kind === "project") return a.id === b.id;
  return false;
}

function readStoredSpine(): Spine | null {
  try {
    const raw = localStorage.getItem(SPINE_STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Spine;
    if (parsed?.kind === "fleet") return { kind: "fleet" };
    if (parsed?.kind === "project" && typeof parsed.id === "string") {
      return { kind: "project", id: parsed.id };
    }
  } catch {
    /* ignore malformed storage */
  }
  return null;
}

function readStoredFacet(): FacetId | null {
  try {
    const raw = localStorage.getItem(FACET_STORAGE_KEY);
    return isFacetId(raw) ? raw : null;
  } catch {
    return null;
  }
}

/**
 * Resolve a desired spine against the live corpus list. project-as-spine
 * default: land in a project; fall back to Fleet only when there are none. A
 * persisted project that no longer exists falls back to the first project (or
 * Fleet on a cold install).
 */
function resolveSpine(want: Spine | null, corpora: CorpusInfo[]): Spine {
  if (want?.kind === "project" && corpora.some((c) => c.id === want.id)) {
    return want;
  }
  if (want?.kind === "fleet") return { kind: "fleet" };
  return corpora.length > 0
    ? { kind: "project", id: corpora[0].id }
    : { kind: "fleet" };
}

/**
 * The single shared workspace context — the spine (selected once) plus the
 * active facet. Receives `corpora` as a prop (rather than calling
 * `useDaemonStatus` itself) so the whole shell stays pure and renders in
 * Storybook against mock fixtures.
 */
export function WorkspaceProvider({
  corpora,
  children,
  initialSpine,
  initialFacet,
}: {
  corpora: CorpusInfo[];
  children: ReactNode;
  /** Override the initial spine (Storybook / testing). */
  initialSpine?: Spine;
  /** Override the initial facet (Storybook / testing). */
  initialFacet?: FacetId;
}) {
  const [spine, setSpine] = useState<Spine>(() =>
    resolveSpine(initialSpine ?? readStoredSpine(), corpora),
  );
  const [facet, setFacetState] = useState<FacetId>(
    () => initialFacet ?? readStoredFacet() ?? "ask",
  );

  // Re-validate the spine when the corpus list changes — a project the spine
  // pointed at may have been removed, or a cold install may have just gained
  // its first project. Never re-selects out from under a still-valid choice.
  useEffect(() => {
    setSpine((prev) => {
      const next = resolveSpine(prev, corpora);
      return sameSpine(prev, next) ? prev : next;
    });
  }, [corpora]);

  // Persist the spine so it carries across launches.
  useEffect(() => {
    try {
      localStorage.setItem(SPINE_STORAGE_KEY, JSON.stringify(spine));
    } catch {
      /* ignore */
    }
  }, [spine]);

  const selectProject = useCallback(
    (id: string) => setSpine({ kind: "project", id }),
    [],
  );
  const selectFleet = useCallback(() => setSpine({ kind: "fleet" }), []);
  const setFacet = useCallback((f: FacetId) => {
    setFacetState(f);
    try {
      localStorage.setItem(FACET_STORAGE_KEY, f);
    } catch {
      /* ignore */
    }
  }, []);

  const value = useMemo<WorkspaceContextValue>(() => {
    const isFleet = spine.kind === "fleet";
    const activeProjectId = isFleet ? null : spine.id;
    const activeProject =
      activeProjectId != null
        ? (corpora.find((c) => c.id === activeProjectId) ?? null)
        : null;
    return {
      spine,
      isFleet,
      activeProjectId,
      activeProject,
      corpora,
      selectProject,
      selectFleet,
      facet,
      setFacet,
    };
  }, [spine, corpora, facet, selectProject, selectFleet, setFacet]);

  return (
    <WorkspaceContext.Provider value={value}>
      {children}
    </WorkspaceContext.Provider>
  );
}

/** Read the one shared workspace context. Throws if used outside the provider. */
export function useWorkspace(): WorkspaceContextValue {
  const ctx = useContext(WorkspaceContext);
  if (!ctx) {
    throw new Error("useWorkspace must be used within a WorkspaceProvider");
  }
  return ctx;
}
