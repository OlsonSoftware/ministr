import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ActivityEvent } from "../lib/types";

/**
 * Per-session activity feed.
 *
 * `recent_activity` already supports `since_ms` server-side, so after the
 * initial 500-event window this polls only the delta. `session_id` is
 * passed through (forward-compatible: it filters server-side once the Rust
 * step lands; until then we also filter client-side, so it is correct
 * either way). The hook returns the *full* session-scoped list (newest
 * first) — the timeline owns filtering + progressive reveal, because
 * filtering must apply before pagination.
 */

const INITIAL_LIMIT = 500;
const DEFAULT_POLL_MS = 3000;
const MAX_BACKOFF_MS = 30_000;

function tauriReady(): boolean {
  return Boolean(
    (window as unknown as { __TAURI_INTERNALS__?: unknown })
      .__TAURI_INTERNALS__,
  );
}

function key(e: ActivityEvent): string {
  return `${e.timestamp_ms}-${e.tool}-${e.corpus_id}`;
}

export interface UseSessionActivity {
  /** Full session-scoped list, newest first. The timeline filters then
   *  paginates this client-side. */
  events: ActivityEvent[];
  loading: boolean;
  error: string | null;
  /** Newest event timestamp from the *previous* snapshot — rows newer
   *  than this just arrived and should flash. */
  flashSince: number;
  refresh: () => void;
}

export function useSessionActivity(
  sessionId: string | null,
  opts: { pollMs?: number; live?: boolean } = {},
): UseSessionActivity {
  const pollMs = opts.pollMs ?? DEFAULT_POLL_MS;
  const live = opts.live ?? true;

  const [events, setEvents] = useState<ActivityEvent[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [flashSince, setFlashSince] = useState(0);

  const seen = useRef<Set<string>>(new Set());
  const newestMs = useRef(0);
  const [reloadKey, setReloadKey] = useState(0); // bump to force re-fetch

  const refresh = useCallback(() => setReloadKey((k) => k + 1), []);

  useEffect(() => {
    if (!sessionId) {
      setEvents([]);
      setLoading(false);
      return;
    }

    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
    let backoff = pollMs;

    // Reset accumulation when the session changes.
    seen.current = new Set();
    newestMs.current = 0;
    setEvents([]);
    setFlashSince(0);
    setLoading(true);
    setError(null);

    const fetchInitial = async () => {
      if (!tauriReady()) {
        if (!cancelled) setError("Tauri IPC bridge not available");
        return;
      }
      try {
        const all = await invoke<ActivityEvent[]>("recent_activity", {
          session_id: sessionId,
          limit: INITIAL_LIMIT,
        });
        if (cancelled) return;
        const mine = all.filter((e) => e.session_id === sessionId);
        for (const e of mine) seen.current.add(key(e));
        newestMs.current = mine.reduce(
          (m, e) => Math.max(m, e.timestamp_ms),
          0,
        );
        setEvents(mine);
        setError(null);
        backoff = pollMs;
      } catch (e) {
        if (!cancelled) setError(String(e));
        backoff = Math.min(backoff * 2, MAX_BACKOFF_MS);
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    const fetchDelta = async () => {
      if (cancelled || !tauriReady()) return;
      try {
        const next = await invoke<ActivityEvent[]>("recent_activity", {
          session_id: sessionId,
          since_ms: newestMs.current,
          limit: INITIAL_LIMIT,
        });
        if (cancelled) return;
        const fresh = next.filter(
          (e) => e.session_id === sessionId && !seen.current.has(key(e)),
        );
        backoff = pollMs;
        if (fresh.length === 0) return;
        setFlashSince(newestMs.current);
        for (const e of fresh) {
          seen.current.add(key(e));
          newestMs.current = Math.max(newestMs.current, e.timestamp_ms);
        }
        // newest first; `recent_activity` returns newest-first already.
        setEvents((prev) => [...fresh, ...prev]);
      } catch {
        backoff = Math.min(backoff * 2, MAX_BACKOFF_MS);
      }
    };

    const loop = async () => {
      if (cancelled) return;
      if (typeof document !== "undefined" && document.hidden) {
        timer = setTimeout(() => void loop(), backoff);
        return;
      }
      await fetchDelta();
      if (!cancelled) timer = setTimeout(() => void loop(), backoff);
    };

    void fetchInitial().then(() => {
      if (!cancelled && live) timer = setTimeout(() => void loop(), pollMs);
    });

    const onVis = () => {
      if (!document.hidden && live && !cancelled) void fetchDelta();
    };
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", onVis);
    }

    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
      if (typeof document !== "undefined") {
        document.removeEventListener("visibilitychange", onVis);
      }
    };
  }, [sessionId, pollMs, live, reloadKey]);

  return { events, loading, error, flashSince, refresh };
}
