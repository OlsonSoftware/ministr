/**
 * Corpus health — derives a project's "is it tended?" status from its index
 * freshness + indexing/error state. The Projects surface uses this to make
 * health the headline: a fresh index reads calm, a stale one nudges a
 * reindex. Pure + null-safe.
 */
import type { Tone } from "./status";
import type { CorpusInfo } from "./types";

export interface CorpusHealth {
  tone: Tone;
  /** Short uppercase word for the headline (FRESH / STALE / INDEXING / …). */
  word: string;
  /** True when the index is fresh enough to trust without a reindex. */
  ok: boolean;
}

const DAY = 86_400; // seconds

/**
 * @param indexing whether an index run is in flight (from the view model /
 *   live progress) — overrides the time-based freshness.
 * @param nowSec injectable clock (seconds) for deterministic stories/tests.
 */
export function corpusHealth(
  corpus: CorpusInfo,
  indexing = corpus.status.state === "indexing",
  nowSec: number = Date.now() / 1000,
): CorpusHealth {
  if (indexing) return { tone: "accent", word: "INDEXING", ok: false };
  if (corpus.status.state === "error")
    return { tone: "danger", word: "INDEX ERROR", ok: false };
  if (!corpus.last_indexed)
    return { tone: "muted", word: "NOT INDEXED", ok: false };

  const age = nowSec - corpus.last_indexed;
  if (age < DAY) return { tone: "success", word: "FRESH", ok: true };
  if (age < 7 * DAY) return { tone: "accent", word: "INDEXED", ok: true };
  return { tone: "warning", word: "STALE", ok: false };
}
