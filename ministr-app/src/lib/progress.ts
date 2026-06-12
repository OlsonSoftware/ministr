import type { IngestionProgressInfo } from "./ipc";

/**
 * Pure derivation math for ingestion progress (gui-progress-data-hook):
 * turns a stream of polled daemon snapshots into the determinate values
 * the progress instruments render — percent, smoothed rate, and an HONEST
 * ETA. No React, no clocks of its own (callers inject `nowMs`), so every
 * rule is unit-testable with deterministic synthetic sequences.
 *
 * Honesty rules (the whole point — a lying countdown is worse than none):
 * - ETA is hidden (null) until the rate has stabilized: at least
 *   MIN_RATE_SAMPLES observations with forward progress.
 * - ETA is clamped to >= 0 and rounds to whole seconds; it never freezes —
 *   a stall (no forward progress for STALL_AFTER_MS) hides it again.
 * - A counter reset (done shrinks, or totals shrink) means a NEW run:
 *   all derived state resets rather than producing a negative rate.
 * - A phase change resets the rate state: files/s and embeddings/s are
 *   different units and must never blend into one EWMA.
 */

/** EWMA smoothing factor for the per-poll rate samples. */
const EWMA_ALPHA = 0.3;
/** Forward-progress samples required before an ETA is shown. */
const MIN_RATE_SAMPLES = 3;
/** No forward progress for this long → the ETA hides (stall). */
const STALL_AFTER_MS = 5_000;
/** Rates below this (units/sec) are treated as no signal. */
const MIN_RATE = 1e-6;

/** What a progress instrument renders for one corpus. */
export interface DerivedProgress {
  corpusId: string;
  /** Daemon phase verbatim ("idle" | "discovering" | "parsing" | "embedding" | "finalizing"). */
  phase: string;
  /** True while the daemon reports the run as running (status 1). */
  running: boolean;
  /** True once the daemon reports the run complete (status 2). */
  complete: boolean;
  filesDone: number;
  filesTotal: number;
  embeddingsDone: number;
  embeddingsTotal: number;
  /** Relative path being processed right now, when known. */
  currentFile: string | null;
  /** Fraction complete in the active phase's unit, 0..1; null when the
   *  total is unknown (early discovery). Complete runs report 1. */
  percent: number | null;
  /** Smoothed forward rate in the active phase's unit per second; null
   *  until there is signal. */
  ratePerSec: number | null;
  /** Whole seconds remaining, clamped >= 0; null until stable, and null
   *  again during a stall. */
  etaSeconds: number | null;
  /** True when progress has not moved for STALL_AFTER_MS while running. */
  stalled: boolean;
}

/** The (done, total) pair the active phase is measured in. */
function phaseCounters(snap: IngestionProgressInfo): {
  done: number;
  total: number;
} {
  if (snap.phase === "embedding" || snap.phase === "finalizing") {
    return { done: snap.embeddings_done, total: snap.embeddings_total };
  }
  return { done: snap.files_done, total: snap.files_total };
}

interface CorpusState {
  phase: string;
  prevDone: number;
  prevAtMs: number;
  ewmaRate: number | null;
  forwardSamples: number;
  lastForwardAtMs: number;
  /** Snapshot identity guard: re-observing the same object (StrictMode
   *  double-render) must not decay the rate with a zero-delta sample. */
  lastSnapshot: IngestionProgressInfo | null;
  lastDerived: DerivedProgress | null;
}

/** Stateful per-corpus tracker. Feed it every polled snapshot with a
 *  timestamp; read back the derived view. */
export class ProgressTracker {
  private corpora = new Map<string, CorpusState>();

  observe(snap: IngestionProgressInfo, nowMs: number): DerivedProgress {
    const prior = this.corpora.get(snap.corpus_id);

    // StrictMode / re-render guard: identical snapshot object → cached result.
    if (prior?.lastSnapshot === snap && prior.lastDerived) {
      return prior.lastDerived;
    }

    const { done, total } = phaseCounters(snap);
    let state = prior;

    const counterReset =
      state !== undefined && state.phase === snap.phase && done < state.prevDone;
    const phaseChanged = state !== undefined && state.phase !== snap.phase;
    if (state === undefined || counterReset || phaseChanged) {
      state = {
        phase: snap.phase,
        prevDone: done,
        prevAtMs: nowMs,
        ewmaRate: null,
        forwardSamples: 0,
        lastForwardAtMs: nowMs,
        lastSnapshot: null,
        lastDerived: null,
      };
      this.corpora.set(snap.corpus_id, state);
    } else {
      const dtMs = nowMs - state.prevAtMs;
      if (dtMs > 0) {
        const delta = done - state.prevDone;
        const rate = (delta * 1000) / dtMs;
        state.ewmaRate =
          state.ewmaRate === null
            ? rate
            : EWMA_ALPHA * rate + (1 - EWMA_ALPHA) * state.ewmaRate;
        if (delta > 0) {
          state.forwardSamples += 1;
          state.lastForwardAtMs = nowMs;
        }
        state.prevDone = done;
        state.prevAtMs = nowMs;
      }
    }

    const running = snap.status === 1;
    const complete = snap.status === 2;
    const stalled =
      running && nowMs - state.lastForwardAtMs >= STALL_AFTER_MS;

    let percent: number | null = null;
    if (complete) {
      percent = 1;
    } else if (total > 0) {
      percent = Math.min(1, Math.max(0, done / total));
    }

    const rateKnown =
      state.ewmaRate !== null && state.ewmaRate > MIN_RATE;
    const ratePerSec = rateKnown ? state.ewmaRate : null;

    let etaSeconds: number | null = null;
    if (
      running &&
      !stalled &&
      rateKnown &&
      state.forwardSamples >= MIN_RATE_SAMPLES &&
      total > 0
    ) {
      const remaining = Math.max(0, total - done);
      etaSeconds = Math.max(0, Math.round(remaining / (state.ewmaRate as number)));
    }

    const derived: DerivedProgress = {
      corpusId: snap.corpus_id,
      phase: snap.phase,
      running,
      complete,
      filesDone: snap.files_done,
      filesTotal: snap.files_total,
      embeddingsDone: snap.embeddings_done,
      embeddingsTotal: snap.embeddings_total,
      currentFile: snap.current_file ? snap.current_file : null,
      percent,
      ratePerSec,
      etaSeconds,
      stalled,
    };
    state.lastSnapshot = snap;
    state.lastDerived = derived;
    return derived;
  }
}
