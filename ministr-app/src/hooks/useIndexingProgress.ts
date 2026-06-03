/**
 * useIndexingProgress — subscribe to live indexing-progress events.
 *
 * Wraps the Tauri `indexing_progress_events` Channel so a surface can
 * read per-corpus progress without polling. Backed by a tokio task on
 * the Rust side that pushes one event per change (status flip, file
 * tick, current-file change) plus a synthetic ETA in seconds.
 *
 * One shared subscription: the Channel + backend task are opened ONCE
 * (lazily, on first subscriber) into a module-level store, and every
 * surface reads from it via `useSyncExternalStore`. Previously each call
 * site spawned its own Channel + backend task; with the view model now
 * adopted on several always-mounted surfaces that meant N redundant
 * streams. The store keeps the original no-teardown liveness: the single
 * channel lives for the app's lifetime (the backend exits on its own if
 * the channel is ever GC'd).
 *
 * Usage:
 *   const progress = useIndexingProgress();
 *   const p = progress[corpusId];
 *   if (p?.status === 1) { ...show progress... }
 */
import { useSyncExternalStore } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";

export interface IndexingProgressEvent {
  corpus_id: string;
  /** 0 = pending, 1 = running, 2 = complete. Mirrors `IngestionProgress`. */
  status: number;
  phase: string;
  files_total: number;
  files_done: number;
  sections_done: number;
  embeddings_total: number;
  embeddings_done: number;
  current_file: string;
  /** Crude rate-based ETA. `null` until at least one second of running
   *  samples has been observed. */
  estimated_remaining_secs: number | null;
  timestamp_ms: number;
}

type ProgressMap = Record<string, IndexingProgressEvent>;

// ── Shared module store ──────────────────────────────────────────────────────
//
// `useSyncExternalStore` requires a stable snapshot reference between renders
// (it compares with Object.is), so `snapshot` is only ever reassigned when a
// new event arrives, and the initial value is a frozen constant.

const EMPTY: ProgressMap = Object.freeze({}) as ProgressMap;

let snapshot: ProgressMap = EMPTY;
const listeners = new Set<() => void>();
let started = false;

/** Open the single backend Channel + task, once. */
function ensureStarted(): void {
  if (started) return;
  started = true;

  const channel = new Channel<IndexingProgressEvent>();
  channel.onmessage = (event) => {
    // New object ref so useSyncExternalStore sees a change and re-renders
    // every subscribed surface.
    snapshot = { ...snapshot, [event.corpus_id]: event };
    for (const notify of listeners) notify();
  };

  invoke("indexing_progress_events", { onEvent: channel }).catch(() => {
    // If the command fails to spawn we just don't get live progress;
    // daemon_status still reports indexing state at coarser granularity.
  });
}

function subscribe(onStoreChange: () => void): () => void {
  ensureStarted();
  listeners.add(onStoreChange);
  return () => {
    listeners.delete(onStoreChange);
    // Intentionally keep the channel open for the app's lifetime — the
    // stream is cheap when idle and reused by the next subscriber.
  };
}

function getSnapshot(): ProgressMap {
  return snapshot;
}

/**
 * Returns the latest progress event keyed by corpus_id. Reactive — every
 * incoming event re-renders all subscribers. All callers share ONE backend
 * subscription via the module store above.
 */
export function useIndexingProgress(): ProgressMap {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
