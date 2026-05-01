import { useEffect, useState } from "react";
import type { CorpusInfo, DaemonStatus } from "../lib/types";

const STORAGE_KEY = "ministr-active-corpus";

/**
 * Global active-corpus selection.
 *
 * The active corpus is the single source of truth for every page that
 * scopes its data by corpus (Search, Symbols, Bridge, Structure, Sessions).
 * Per-page CorpusSelect dropdowns are removed in favor of the shell-level
 * pill that calls `setActiveCorpusId`.
 *
 * On mount: reads localStorage; falls back to the first available corpus.
 * If the persisted id no longer exists in the daemon's registry it falls
 * back to the first available corpus and rewrites storage.
 */
export function useCorpusContext(status: DaemonStatus | null) {
  const [activeCorpusId, setActiveCorpusIdRaw] = useState<string | null>(() => {
    try {
      return localStorage.getItem(STORAGE_KEY);
    } catch {
      return null;
    }
  });

  function setActiveCorpusId(id: string | null) {
    setActiveCorpusIdRaw(id);
    try {
      if (id) localStorage.setItem(STORAGE_KEY, id);
      else localStorage.removeItem(STORAGE_KEY);
    } catch {
      /* ignore */
    }
  }

  useEffect(() => {
    if (!status) return;
    const corpora = status.corpora;
    if (corpora.length === 0) {
      if (activeCorpusId !== null) setActiveCorpusId(null);
      return;
    }
    const exists =
      activeCorpusId !== null &&
      corpora.some((c) => c.id === activeCorpusId);
    if (!exists) {
      setActiveCorpusId(corpora[0].id);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [status?.corpora]);

  const activeCorpus: CorpusInfo | null =
    status?.corpora.find((c) => c.id === activeCorpusId) ?? null;

  return { activeCorpusId, activeCorpus, setActiveCorpusId };
}
