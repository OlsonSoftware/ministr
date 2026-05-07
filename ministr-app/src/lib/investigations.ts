/**
 * Investigation state — a thread of related queries against a corpus, with
 * pinned source sections that persist across queries until the user clears
 * them or switches investigations.
 *
 * Backed by localStorage in v1 (the Tauri store plugin is not yet wired
 * for this app). State is keyed by corpus so switching corpora doesn't
 * leak pinned sources between projects.
 */

const STORAGE_KEY = "ministr:investigations:v1";
const MAX_INVESTIGATIONS = 20;
const MAX_HISTORY_PER_INVESTIGATION = 50;

export interface QueryHistoryEntry {
  query: string;
  ts: number;
  /** Cached?  Only set when the answer landed via cache_hit. */
  cached?: boolean;
}

export interface Investigation {
  id: string;
  corpusId: string;
  /** Display title — defaults to the first query, user-editable. */
  title: string;
  /** Section content_ids pinned to the right pane. Order = pin order. */
  pinnedSourceIds: string[];
  history: QueryHistoryEntry[];
  createdAt: number;
  updatedAt: number;
}

export interface InvestigationStore {
  investigations: Investigation[];
  /** The currently active investigation id, or null = no active. */
  activeId: string | null;
}

// ─────────────────────────────────────────────────────────────────────────────
// Storage

function loadStore(): InvestigationStore {
  if (typeof window === "undefined") {
    return { investigations: [], activeId: null };
  }
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return { investigations: [], activeId: null };
    const parsed = JSON.parse(raw) as Partial<InvestigationStore>;
    return {
      investigations: Array.isArray(parsed.investigations)
        ? parsed.investigations.filter(isValidInvestigation)
        : [],
      activeId: typeof parsed.activeId === "string" ? parsed.activeId : null,
    };
  } catch {
    return { investigations: [], activeId: null };
  }
}

function saveStore(store: InvestigationStore): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(store));
  } catch {
    /* quota / disabled storage — silently no-op */
  }
  // Broadcast so all useInvestigations() instances re-snapshot. Plain
  // localStorage doesn't fire its `storage` event in the originating tab,
  // so we use a custom DOM event instead.
  window.dispatchEvent(new CustomEvent(INVESTIGATIONS_CHANGED));
}

/** Event name dispatched on every investigation-store mutation. */
export const INVESTIGATIONS_CHANGED = "ministr:investigations:changed";

function isValidInvestigation(x: unknown): x is Investigation {
  if (typeof x !== "object" || x === null) return false;
  const r = x as Record<string, unknown>;
  return (
    typeof r.id === "string" &&
    typeof r.corpusId === "string" &&
    typeof r.title === "string" &&
    Array.isArray(r.pinnedSourceIds) &&
    Array.isArray(r.history)
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Public helpers

export function getStore(): InvestigationStore {
  return loadStore();
}

export function listInvestigations(corpusId: string | null): Investigation[] {
  const store = loadStore();
  if (corpusId === null) return store.investigations;
  return store.investigations.filter((inv) => inv.corpusId === corpusId);
}

export function getActiveInvestigation(
  corpusId: string | null,
): Investigation | null {
  const store = loadStore();
  if (!store.activeId) return null;
  const inv = store.investigations.find((i) => i.id === store.activeId);
  if (!inv) return null;
  if (corpusId !== null && inv.corpusId !== corpusId) return null;
  return inv;
}

export function newInvestigation(
  corpusId: string,
  title?: string,
): Investigation {
  const inv: Investigation = {
    id: makeId(),
    corpusId,
    title: title ?? "Untitled investigation",
    pinnedSourceIds: [],
    history: [],
    createdAt: Date.now(),
    updatedAt: Date.now(),
  };
  const store = loadStore();
  store.investigations = [inv, ...store.investigations].slice(
    0,
    MAX_INVESTIGATIONS,
  );
  store.activeId = inv.id;
  saveStore(store);
  return inv;
}

export function setActiveInvestigation(id: string | null): void {
  const store = loadStore();
  store.activeId = id;
  saveStore(store);
}

export function closeInvestigation(id: string): void {
  const store = loadStore();
  store.investigations = store.investigations.filter((i) => i.id !== id);
  if (store.activeId === id) {
    store.activeId = store.investigations[0]?.id ?? null;
  }
  saveStore(store);
}

export function renameInvestigation(id: string, title: string): void {
  mutate(id, (inv) => ({ ...inv, title, updatedAt: Date.now() }));
}

export function pinSource(investigationId: string, sourceId: string): void {
  mutate(investigationId, (inv) => {
    if (inv.pinnedSourceIds.includes(sourceId)) return inv;
    return {
      ...inv,
      pinnedSourceIds: [...inv.pinnedSourceIds, sourceId],
      updatedAt: Date.now(),
    };
  });
}

export function unpinSource(investigationId: string, sourceId: string): void {
  mutate(investigationId, (inv) => ({
    ...inv,
    pinnedSourceIds: inv.pinnedSourceIds.filter((s) => s !== sourceId),
    updatedAt: Date.now(),
  }));
}

export function clearPinnedSources(investigationId: string): void {
  mutate(investigationId, (inv) => ({
    ...inv,
    pinnedSourceIds: [],
    updatedAt: Date.now(),
  }));
}

export function recordQuery(
  investigationId: string,
  query: string,
  cached?: boolean,
): void {
  mutate(investigationId, (inv) => {
    const entry: QueryHistoryEntry = { query, ts: Date.now(), cached };
    // Auto-title the investigation from the first query.
    const title =
      inv.history.length === 0 && inv.title === "Untitled investigation"
        ? query.slice(0, 80)
        : inv.title;
    return {
      ...inv,
      title,
      history: [entry, ...inv.history].slice(0, MAX_HISTORY_PER_INVESTIGATION),
      updatedAt: Date.now(),
    };
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Internals

function mutate(
  id: string,
  fn: (inv: Investigation) => Investigation,
): void {
  const store = loadStore();
  const idx = store.investigations.findIndex((i) => i.id === id);
  if (idx < 0) return;
  store.investigations[idx] = fn(store.investigations[idx]);
  saveStore(store);
}

function makeId(): string {
  // Short, sortable-ish, collision-safe enough for client-only use.
  return (
    Date.now().toString(36) +
    Math.random().toString(36).slice(2, 8)
  );
}
