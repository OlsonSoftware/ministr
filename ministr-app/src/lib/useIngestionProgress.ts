import { useMemo, useRef } from "react";
import { ingestionProgress } from "./ipc";
import { type DerivedProgress, ProgressTracker } from "./progress";
import { usePoll } from "./usePoll";

/**
 * Live per-corpus ingestion progress with client-derived rate + honest ETA
 * (gui-progress-data-hook). Polls the daemon's progress snapshot and feeds a
 * [`ProgressTracker`], whose honesty rules (ETA hidden until stable, clamped,
 * stall-aware; resets on new runs and phase changes) are unit-tested in
 * lib/progress.test.ts. The tracker lives in a ref and is identity-guarded,
 * so StrictMode double-renders cannot decay the rate.
 */
export function useIngestionProgress(intervalMs = 1000): {
  /** Derived progress keyed by corpus id; empty until the first poll lands. */
  progress: Map<string, DerivedProgress>;
  error: string | null;
} {
  const { data, error } = usePoll(ingestionProgress, intervalMs);
  const tracker = useRef<ProgressTracker | null>(null);
  tracker.current ??= new ProgressTracker();

  const progress = useMemo(() => {
    const out = new Map<string, DerivedProgress>();
    if (data) {
      const now = Date.now();
      for (const snap of data) {
        out.set(snap.corpus_id, tracker.current!.observe(snap, now));
      }
    }
    return out;
  }, [data]);

  return { progress, error };
}
