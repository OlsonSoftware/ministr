import type { ActivityEvent } from "./ipc";
import { activitySentence } from "./receipts";

/**
 * Presence derivation (blueprint §3.2, DESIGN invariant 2: presence is
 * REAL — no event, no line). Pure function over the activity ring with
 * an injected clock so the window is testable.
 */

export const LIVE_WINDOW_MS = 15_000;
export const RECENT_WINDOW_MS = 10 * 60_000;

export type Presence =
  | { kind: "live"; sentence: string }
  | { kind: "recent"; sentence: string }
  | null;

export function derivePresence(
  events: ActivityEvent[],
  corpusId: string,
  nowMs: number,
): Presence {
  const latest = events
    .filter((e) => e.corpus_id === corpusId)
    .reduce<ActivityEvent | null>(
      (a, b) => (a === null || b.timestamp_ms > a.timestamp_ms ? b : a),
      null,
    );
  if (!latest) return null;

  const age = nowMs - latest.timestamp_ms;
  if (age <= LIVE_WINDOW_MS) {
    return { kind: "live", sentence: activitySentence(latest) };
  }
  if (age <= RECENT_WINDOW_MS) {
    const mins = Math.max(1, Math.round(age / 60_000));
    return {
      kind: "recent",
      sentence: `your AI last looked at this project ${mins} minute${mins === 1 ? "" : "s"} ago`,
    };
  }
  return null;
}
