/**
 * useIndexingProgress — subscribe to live indexing-progress events.
 *
 * Wraps the Tauri `indexing_progress_events` Channel so a surface can
 * read per-corpus progress without polling. Backed by a tokio task on
 * the Rust side that pushes one event per change (status flip, file
 * tick, current-file change) plus a synthetic ETA in seconds.
 *
 * Usage:
 *   const progress = useIndexingProgress();
 *   const p = progress[corpusId];
 *   if (p?.status === 1) { ...show progress... }
 */
import { useEffect, useRef, useState } from "react";
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

/**
 * Returns the latest progress event keyed by corpus_id. Reactive — every
 * incoming event triggers a setState that components observe via the
 * returned object reference.
 */
export function useIndexingProgress(): Record<string, IndexingProgressEvent> {
  const [byCorpus, setByCorpus] = useState<
    Record<string, IndexingProgressEvent>
  >({});
  // Held only so React's strict-mode double-invoke doesn't double-spawn
  // backend tasks; we intentionally don't tear down on unmount because the
  // backend exits on its own when send() fails (channel GC'd).
  const subscribed = useRef(false);

  useEffect(() => {
    if (subscribed.current) return;
    subscribed.current = true;

    const channel = new Channel<IndexingProgressEvent>();
    channel.onmessage = (event) => {
      setByCorpus((prev) => ({ ...prev, [event.corpus_id]: event }));
    };

    invoke("indexing_progress_events", { onEvent: channel }).catch(() => {
      // If the command fails to spawn we just don't get live progress;
      // daemon_status still reports indexing state at coarser granularity.
    });
  }, []);

  return byCorpus;
}
