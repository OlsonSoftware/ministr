import { useCallback, useEffect, useState } from "react";
import {
  INVESTIGATIONS_CHANGED,
  type Investigation,
  closeInvestigation as storeClose,
  getStore,
  listInvestigations,
  newInvestigation as storeNew,
  pinSource as storePin,
  recordQuery as storeRecord,
  renameInvestigation as storeRename,
  setActiveInvestigation as storeSetActive,
  unpinSource as storeUnpin,
  clearPinnedSources as storeClearPins,
} from "../lib/investigations";

/**
 * React binding for the investigation store.
 *
 * The store is plain localStorage, so we cache snapshots in component state
 * and re-read on every mutation. Cross-tab sync isn't a concern (Tauri
 * single-window). All mutations route through this hook so the snapshot
 * stays in lockstep with persistence.
 */
export function useInvestigations(corpusId: string | null) {
  const [snapshot, setSnapshot] = useState(() => getStore());

  // Re-derive when corpus changes — listInvestigations filters per-corpus.
  const investigations = listInvestigations(corpusId);
  const active =
    snapshot.activeId !== null
      ? investigations.find((i) => i.id === snapshot.activeId) ?? null
      : null;

  const refresh = useCallback(() => setSnapshot(getStore()), []);

  // Cross-instance sync: any mutation in any useInvestigations() consumer
  // dispatches INVESTIGATIONS_CHANGED, so all peers re-snapshot together.
  useEffect(() => {
    function onChange() {
      setSnapshot(getStore());
    }
    window.addEventListener(INVESTIGATIONS_CHANGED, onChange);
    return () => window.removeEventListener(INVESTIGATIONS_CHANGED, onChange);
  }, []);

  // If the active investigation belongs to a different corpus, clear it
  // so the workspace doesn't show stale pinned sources.
  useEffect(() => {
    if (snapshot.activeId === null) return;
    const inv = snapshot.investigations.find((i) => i.id === snapshot.activeId);
    if (!inv) {
      storeSetActive(null);
      refresh();
      return;
    }
    if (corpusId !== null && inv.corpusId !== corpusId) {
      // Switch to the most recent investigation for the new corpus, if any.
      const fallback = listInvestigations(corpusId)[0] ?? null;
      storeSetActive(fallback?.id ?? null);
      refresh();
    }
  }, [corpusId, snapshot, refresh]);

  const create = useCallback(
    (title?: string): Investigation | null => {
      if (!corpusId) return null;
      const inv = storeNew(corpusId, title);
      refresh();
      return inv;
    },
    [corpusId, refresh],
  );

  const setActive = useCallback(
    (id: string | null) => {
      storeSetActive(id);
      refresh();
    },
    [refresh],
  );

  const close = useCallback(
    (id: string) => {
      storeClose(id);
      refresh();
    },
    [refresh],
  );

  const rename = useCallback(
    (id: string, title: string) => {
      storeRename(id, title);
      refresh();
    },
    [refresh],
  );

  const pin = useCallback(
    (sourceId: string) => {
      // Lazy-create an investigation if there isn't one — the user is
      // signalling intent by pinning, even if they haven't asked yet.
      let target = active;
      if (!target && corpusId) {
        target = storeNew(corpusId);
      }
      if (target) {
        storePin(target.id, sourceId);
        refresh();
      }
    },
    [active, corpusId, refresh],
  );

  const unpin = useCallback(
    (sourceId: string) => {
      if (active) {
        storeUnpin(active.id, sourceId);
        refresh();
      }
    },
    [active, refresh],
  );

  const clearPins = useCallback(() => {
    if (active) {
      storeClearPins(active.id);
      refresh();
    }
  }, [active, refresh]);

  const recordQuery = useCallback(
    (query: string, cached?: boolean) => {
      let target = active;
      if (!target && corpusId) {
        target = storeNew(corpusId);
      }
      if (target) {
        storeRecord(target.id, query, cached);
        refresh();
      }
    },
    [active, corpusId, refresh],
  );

  return {
    investigations,
    active,
    pinnedSourceIds: active?.pinnedSourceIds ?? [],
    create,
    setActive,
    close,
    rename,
    pin,
    unpin,
    clearPins,
    recordQuery,
    isPinned: useCallback(
      (sourceId: string) =>
        active ? active.pinnedSourceIds.includes(sourceId) : false,
      [active],
    ),
  };
}
