/**
 * corpusFleet — the single source of truth for per-corpus indexing state.
 *
 * The app has two raw sources for a corpus's state:
 *   1. `DaemonStatus.corpora` (`CorpusInfo[]`) — polled, persisted facts:
 *      files_indexed, sections, vectors, symbols, last_indexed, status.
 *   2. `useIndexingProgress` — the live Tauri Channel: phase + granular
 *      files/sections/embeddings ticks + current file + ETA while indexing.
 *
 * Historically every surface merged these two ad-hoc, so the same corpus
 * looked different from card to card and the "what's happening right now"
 * story was inconsistent. This module collapses both into ONE normalized
 * {@link CorpusViewModel} that every indexing/corpus-state component consumes.
 * Merge logic lives here (pure + testable); components only render.
 */
import { useMemo } from "react";

import {
  useIndexingProgress,
  type IndexingProgressEvent,
} from "../hooks/useIndexingProgress";
import { corpusLabel, corpusRoot } from "./corpus";
import type { CorpusInfo } from "./types";

/** The pipeline phase, normalized. Mirrors `ministr_core::IngestionPhase`
 *  plus the lifecycle states the daemon reports outside an active run. */
export type IndexPhase =
  | "idle"
  | "queued"
  | "discovering"
  | "parsing"
  | "embedding"
  | "finalizing"
  | "ready"
  | "error";

/** A done/total pair for a progress metric. */
export interface MetricPair {
  done: number;
  total: number;
  /** 0–100, clamped. `0` when `total` is 0. */
  pct: number;
}

/** Lifecycle state, collapsed from `IndexingStatus` + persisted counts. */
export type CorpusLifecycle = "idle" | "queued" | "indexing" | "ready" | "error";

/**
 * The normalized, render-ready view of a corpus — the single source of truth
 * every indexing/corpus-state component should consume.
 */
export interface CorpusViewModel {
  id: string;
  label: string;
  root: string;
  model?: string;

  lifecycle: CorpusLifecycle;
  phase: IndexPhase;
  isIndexing: boolean;
  errorMessage?: string;

  // ── Live, while indexing ──
  files: MetricPair;
  /** Sections parsed so far this run (no known total mid-run). */
  sections: number;
  vectors: MetricPair; // embeddings
  currentFile: string;
  etaSecs: number | null;
  /** Items/sec of the active phase, derived from `etaSecs` (parse files/sec
   *  while parsing, vectors/sec while embedding). `null` until an ETA exists. */
  rate: number | null;
  /**
   * Phase-aware "hero" metric for a single primary bar: files while parsing,
   * vectors while embedding — i.e. whichever is actually moving (the parser is
   * backpressured by the bounded parse→embed channel during the GPU phase).
   */
  primary: {
    label: string;
    unit: "files" | "vectors";
    done: number;
    total: number;
    pct: number;
  };

  // ── Persisted / ready stats ──
  filesIndexed: number;
  sectionsIndexed: number;
  vectorsIndexed: number;
  symbols: number;
  sessions: number;
  lastIndexed?: number;

  // ── Escape hatches ──
  corpus: CorpusInfo;
  progress?: IndexingProgressEvent;
}

const ACTIVE_PHASES = ["discovering", "parsing", "embedding", "finalizing"];

function clampPct(done: number, total: number): number {
  if (total <= 0) return 0;
  return Math.min(100, Math.max(0, (done / total) * 100));
}

function pair(done: number, total: number): MetricPair {
  return { done, total, pct: clampPct(done, total) };
}

/** Human label for a phase (Title Case). */
export function phaseLabel(phase: IndexPhase): string {
  switch (phase) {
    case "discovering":
      return "Discovering";
    case "parsing":
      return "Parsing";
    case "embedding":
      return "Embedding";
    case "finalizing":
      return "Finalizing";
    case "queued":
      return "Queued";
    case "ready":
      return "Ready";
    case "error":
      return "Error";
    default:
      return "Idle";
  }
}

/** Merge one corpus + its (optional) live progress event into a view model. */
export function toCorpusViewModel(
  corpus: CorpusInfo,
  progress?: IndexingProgressEvent,
): CorpusViewModel {
  const status = corpus.status;
  const live = progress && progress.status === 1 ? progress : undefined;
  const isIndexing = status.state === "indexing" || live !== undefined;
  const ready = status.state === "idle" && corpus.files_indexed > 0;

  const lifecycle: CorpusLifecycle =
    status.state === "error"
      ? "error"
      : status.state === "queued"
        ? "queued"
        : isIndexing
          ? "indexing"
          : ready
            ? "ready"
            : "idle";

  const rawPhase = (live?.phase ?? "").toLowerCase();
  const phase: IndexPhase =
    status.state === "error"
      ? "error"
      : status.state === "queued" && !live
        ? "queued"
        : isIndexing
          ? ((ACTIVE_PHASES.includes(rawPhase) ? rawPhase : "parsing") as IndexPhase)
          : ready
            ? "ready"
            : "idle";

  const filesDone =
    live?.files_done ?? (status.state === "indexing" ? status.files_done : 0);
  const filesTotal =
    live?.files_total ?? (status.state === "indexing" ? status.files_total : 0);
  const vecDone = live?.embeddings_done ?? 0;
  const vecTotal = live?.embeddings_total ?? 0;

  const onEmbed = phase === "embedding" && vecTotal > 0;
  const primary = onEmbed
    ? {
        label: "Embedding",
        unit: "vectors" as const,
        done: vecDone,
        total: vecTotal,
        pct: clampPct(vecDone, vecTotal),
      }
    : {
        label: phaseLabel(phase),
        unit: "files" as const,
        done: filesDone,
        total: filesTotal,
        pct: clampPct(filesDone, filesTotal),
      };

  const etaSecs = live?.estimated_remaining_secs ?? null;
  const remaining = Math.max(0, primary.total - primary.done);
  const rate = etaSecs && etaSecs > 0 ? remaining / etaSecs : null;

  return {
    id: corpus.id,
    label: corpusLabel(corpus),
    root: corpusRoot(corpus.paths),
    model: corpus.model,
    lifecycle,
    phase,
    isIndexing,
    errorMessage: status.state === "error" ? status.message : undefined,
    files: pair(filesDone, filesTotal),
    sections: live?.sections_done ?? 0,
    vectors: pair(vecDone, vecTotal),
    currentFile: live?.current_file ?? "",
    etaSecs,
    rate,
    primary,
    filesIndexed: corpus.files_indexed,
    sectionsIndexed: corpus.sections_count,
    vectorsIndexed: corpus.embeddings_count,
    symbols: corpus.symbols_count,
    sessions: corpus.active_sessions,
    lastIndexed: corpus.last_indexed,
    corpus,
    progress: live,
  };
}

/** Merge the whole corpus list against the live-progress map. Pure. */
export function mergeCorpusFleet(
  corpora: CorpusInfo[],
  progress: Record<string, IndexingProgressEvent>,
): CorpusViewModel[] {
  return corpora.map((c) => toCorpusViewModel(c, progress[c.id]));
}

/**
 * The single source of truth hook: owns the live-progress subscription and
 * returns the normalized fleet plus an id→model index. Every corpus/indexing
 * surface should call this instead of merging the raw sources itself.
 */
export function useCorpusFleet(corpora: CorpusInfo[]): {
  fleet: CorpusViewModel[];
  byId: Record<string, CorpusViewModel>;
} {
  const progress = useIndexingProgress();
  return useMemo(() => {
    const fleet = mergeCorpusFleet(corpora, progress);
    const byId: Record<string, CorpusViewModel> = {};
    for (const vm of fleet) byId[vm.id] = vm;
    return { fleet, byId };
  }, [corpora, progress]);
}
