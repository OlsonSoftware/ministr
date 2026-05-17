import { useMemo, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { SessionDetail } from "../lib/types";
import type { SessionSample } from "../lib/sessions";
import { recordEndedSession } from "./useSessionHistory";

/**
 * The single source of truth for live sessions.
 *
 * Before this, `SessionView`, `ProjectSessions` and `CorpusView` each
 * polled `list_sessions` independently (and the drawer didn't poll at
 * all). This is one module-level store behind `useSyncExternalStore`: a
 * single `list_sessions` round-trip every {@link BASE_MS} shared by every
 * mounted consumer, with visibility-pause, exponential backoff, a stale
 * flag (keeps last data instead of blanking), per-session turn-diff for
 * fresh-flash, and a bounded per-session sample ring that feeds the
 * drawer's trend sparklines (the daemon keeps no per-session time series).
 */

const BASE_MS = 1500;
const MAX_MS = 30_000;
const MAX_SAMPLES = 240; // ≈6 min @1.5s of *changed* polls; idle is skipped
const ENDED_AFTER_MISSED = 2; // consecutive missing polls ⇒ session ended

export interface SessionsSnapshot {
  /** Newest `list_sessions` result. Unchanged sessions keep a stable
   *  object reference across polls so memoised cards skip re-render. */
  sessions: readonly SessionDetail[];
  byId: ReadonlyMap<string, SessionDetail>;
  /** Per-session poll-sampled ring for sparklines. */
  samples: ReadonlyMap<string, readonly SessionSample[]>;
  /** Session ids whose `current_turn` advanced on the latest poll. */
  freshIds: ReadonlySet<string>;
  /** First poll (success or failure) has completed. */
  loaded: boolean;
  /** Last poll error; previous `sessions` are retained alongside it. */
  error: string | null;
  /** A poll has failed after a prior success — data is last-known. */
  stale: boolean;
  /** Epoch ms of the last *successful* poll (drives the heartbeat). */
  lastSyncMs: number;
}

const EMPTY: SessionsSnapshot = {
  sessions: [],
  byId: new Map(),
  samples: new Map(),
  freshIds: new Set(),
  loaded: false,
  error: null,
  stale: false,
  lastSyncMs: 0,
};

let snapshot: SessionsSnapshot = EMPTY;
const listeners = new Set<() => void>();

let timer: ReturnType<typeof setTimeout> | null = null;
let backoff = BASE_MS;
let running = false;
let tauriReady = false;

const prevTurns = new Map<string, number>();
const missed = new Map<string, number>();
const ring = new Map<string, SessionSample[]>();

function getSnapshot(): SessionsSnapshot {
  return snapshot;
}

function emit(next: SessionsSnapshot): void {
  snapshot = next;
  for (const l of listeners) l();
}

async function waitForTauri(timeoutMs = 5000): Promise<boolean> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (
      (window as unknown as { __TAURI_INTERNALS__?: unknown })
        .__TAURI_INTERNALS__
    ) {
      return true;
    }
    await new Promise((r) => setTimeout(r, 50));
  }
  return false;
}

/** Field-wise equality so an unchanged session keeps its object identity
 *  (the lever that lets `React.memo`'d cards skip re-render under poll). */
function sameSession(a: SessionDetail, b: SessionDetail): boolean {
  return (
    a.session_id === b.session_id &&
    a.current_turn === b.current_turn &&
    a.tokens_used === b.tokens_used &&
    a.tokens_remaining === b.tokens_remaining &&
    a.utilization === b.utilization &&
    a.pressure_level === b.pressure_level &&
    a.delivered_count === b.delivered_count &&
    a.total_deliveries === b.total_deliveries &&
    a.cumulative_tokens_delivered === b.cumulative_tokens_delivered &&
    a.total_tokens_saved === b.total_tokens_saved &&
    a.total_evictions === b.total_evictions &&
    a.total_compressions === b.total_compressions &&
    a.dedup_hits === b.dedup_hits &&
    a.compression_ratio === b.compression_ratio &&
    a.parent_session_id === b.parent_session_id &&
    a.client_name === b.client_name
  );
}

function ingest(list: SessionDetail[]): void {
  const now = Date.now();
  const prevById = snapshot.byId;

  // Referential reuse + change detection.
  const byId = new Map<string, SessionDetail>();
  let listChanged = list.length !== prevById.size;
  const freshIds = new Set<string>();
  for (const incoming of list) {
    const prev = prevById.get(incoming.session_id);
    const stable = prev && sameSession(prev, incoming) ? prev : incoming;
    if (stable !== prev) listChanged = true;
    byId.set(incoming.session_id, stable);

    const prevTurn = prevTurns.get(incoming.session_id);
    if (prevTurn !== undefined && incoming.current_turn > prevTurn) {
      freshIds.add(incoming.session_id);
    }
    prevTurns.set(incoming.session_id, incoming.current_turn);

    // Sample ring — only record on a meaningful change to bound growth.
    const r = ring.get(incoming.session_id);
    const last = r?.[r.length - 1];
    if (!last || last.tokensUsed !== incoming.tokens_used) {
      const nextRing = (r ?? []).concat({
        t: now,
        tokensUsed: incoming.tokens_used,
        utilization: incoming.utilization,
        turn: incoming.current_turn,
      });
      if (nextRing.length > MAX_SAMPLES) nextRing.splice(0, nextRing.length - MAX_SAMPLES);
      ring.set(incoming.session_id, nextRing);
    }
    missed.delete(incoming.session_id);
  }

  // Ended detection — gone for ≥N consecutive polls.
  for (const [id, prev] of prevById) {
    if (byId.has(id)) continue;
    const n = (missed.get(id) ?? 0) + 1;
    if (n >= ENDED_AFTER_MISSED) {
      recordEndedSession(prev);
      missed.delete(id);
      prevTurns.delete(id);
      ring.delete(id);
    } else {
      missed.set(id, n);
    }
  }

  const samples = new Map<string, readonly SessionSample[]>();
  for (const [id, r] of ring) samples.set(id, r);

  emit({
    sessions: listChanged ? Array.from(byId.values()) : snapshot.sessions,
    byId,
    samples,
    freshIds,
    loaded: true,
    error: null,
    stale: false,
    lastSyncMs: now,
  });
}

async function pollOnce(): Promise<void> {
  if (typeof document !== "undefined" && document.hidden) {
    timer = null; // paused; visibilitychange resumes immediately
    return;
  }
  if (!tauriReady) {
    tauriReady = await waitForTauri();
    if (!tauriReady) {
      emit({
        ...snapshot,
        loaded: true,
        error: "Tauri IPC bridge not available",
        stale: snapshot.lastSyncMs > 0,
      });
      schedule(MAX_MS);
      return;
    }
  }
  try {
    const list = await invoke<SessionDetail[]>("list_sessions");
    backoff = BASE_MS;
    ingest(list);
  } catch (e) {
    const msg = String(e);
    console.error("[ministr] list_sessions failed:", msg);
    backoff = Math.min(backoff * 2, MAX_MS);
    emit({
      ...snapshot,
      loaded: true,
      error: msg,
      stale: snapshot.lastSyncMs > 0,
    });
  } finally {
    schedule(backoff);
  }
}

function schedule(ms: number): void {
  if (timer) clearTimeout(timer);
  timer = setTimeout(() => void pollOnce(), ms);
}

function onVisibility(): void {
  if (running && !document.hidden && timer === null) {
    void pollOnce();
  }
}

function start(): void {
  if (running) return;
  running = true;
  if (typeof document !== "undefined") {
    document.addEventListener("visibilitychange", onVisibility);
  }
  void pollOnce();
}

function stop(): void {
  running = false;
  if (timer) clearTimeout(timer);
  timer = null;
  if (typeof document !== "undefined") {
    document.removeEventListener("visibilitychange", onVisibility);
  }
}

function subscribe(cb: () => void): () => void {
  listeners.add(cb);
  start();
  return () => {
    listeners.delete(cb);
    if (listeners.size === 0) stop();
  };
}

/** Subscribe to the shared session store. One poll for all consumers. */
export function useSessions(): SessionsSnapshot {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

export interface UseSessionResult {
  session: SessionDetail | null;
  /** Present in the latest live `list_sessions` result. */
  isLive: boolean;
  parent: SessionDetail | null;
  children: SessionDetail[];
  samples: readonly SessionSample[];
  fresh: boolean;
  loaded: boolean;
  error: string | null;
  stale: boolean;
}

/** Derive one session + its lineage + sample ring from the shared store.
 *  Memoised on the stable session reference so the drawer only re-renders
 *  when *this* session actually changed. */
export function useSession(sessionId: string | null): UseSessionResult {
  const snap = useSessions();
  return useMemo(() => {
    const session = sessionId ? (snap.byId.get(sessionId) ?? null) : null;
    const parentId = session?.parent_session_id;
    const parent = parentId ? (snap.byId.get(parentId) ?? null) : null;
    const children = sessionId
      ? snap.sessions.filter((s) => s.parent_session_id === sessionId)
      : [];
    return {
      session,
      isLive: sessionId ? snap.byId.has(sessionId) : false,
      parent,
      children,
      samples: (sessionId && snap.samples.get(sessionId)) || [],
      fresh: sessionId ? snap.freshIds.has(sessionId) : false,
      loaded: snap.loaded,
      error: snap.error,
      stale: snap.stale,
    };
    // snap identity changes per emit; deriving here keeps the memo cheap
    // and the stable session ref keeps downstream memo intact.
  }, [snap, sessionId]);
}
