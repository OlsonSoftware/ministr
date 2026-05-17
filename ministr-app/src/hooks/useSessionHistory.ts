import { useCallback, useEffect, useState } from "react";
import type { SessionDetail } from "../lib/types";

/**
 * 24h ended-session history.
 *
 * `list_sessions` only returns *live* sessions, so once an agent
 * disconnects its last-known `SessionDetail` is gone. The drawer needs it
 * to render a deep-link to an ended session. This persists a small,
 * time-windowed ring under the exact key `usePreferences.resetPreferences`
 * already clears, so "Reset preferences" stays coherent.
 */

const KEY = "ministr-sessions-history-v1";
const WINDOW_MS = 24 * 60 * 60 * 1000;
const MAX_ENTRIES = 200;

export interface EndedSession {
  session: SessionDetail;
  /** Epoch ms when the session was first observed missing. */
  endedAt: number;
}

interface Stored {
  v: 1;
  items: EndedSession[];
}

function prune(items: EndedSession[], now: number): EndedSession[] {
  return items
    .filter((e) => now - e.endedAt < WINDOW_MS)
    .slice(-MAX_ENTRIES);
}

/** Read the pruned history. Tolerates absent / legacy / corrupt values
 *  (discards and starts fresh — never throws). */
export function loadEndedHistory(): EndedSession[] {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as Partial<Stored>;
    if (parsed?.v !== 1 || !Array.isArray(parsed.items)) return [];
    return prune(parsed.items, Date.now());
  } catch {
    return [];
  }
}

/** Append (or refresh) an ended session, pruned + capped. Idempotent per
 *  session id — re-recording updates the snapshot without duplicating. */
export function recordEndedSession(session: SessionDetail): void {
  try {
    const now = Date.now();
    const existing = loadEndedHistory().filter(
      (e) => e.session.session_id !== session.session_id,
    );
    const next: Stored = {
      v: 1,
      items: prune([...existing, { session, endedAt: now }], now),
    };
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    /* ignore — history is best-effort */
  }
}

/** Look up one ended session's last-known detail (drawer seed of last
 *  resort for a reopened, now-gone session). */
export function endedSessionSeed(
  sessionId: string,
): SessionDetail | null {
  return (
    loadEndedHistory().find((e) => e.session.session_id === sessionId)
      ?.session ?? null
  );
}

/** Reactive view of the ended-session ring (newest first). */
export function useSessionHistory() {
  const [ended, setEnded] = useState<EndedSession[]>(() => loadEndedHistory());

  const refresh = useCallback(() => {
    setEnded(loadEndedHistory().slice().reverse());
  }, []);

  useEffect(() => {
    refresh();
    // Other tabs / the store writing in this tab won't fire `storage`
    // for same-tab writes; a light interval keeps it fresh enough for a
    // history list without coupling to the session store internals.
    // ministr lives in the tray, so skip the tick while the window is
    // hidden and catch up the moment it's shown again — consistent with
    // the daemon-status / session pollers.
    const id = window.setInterval(() => {
      if (!document.hidden) refresh();
    }, 5000);
    const onStorage = (e: StorageEvent) => {
      if (e.key === KEY) refresh();
    };
    const onVisibility = () => {
      if (!document.hidden) refresh();
    };
    window.addEventListener("storage", onStorage);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      clearInterval(id);
      window.removeEventListener("storage", onStorage);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [refresh]);

  return { ended, refresh };
}
