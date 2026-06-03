/**
 * Conversation thread model for the Ask surface.
 *
 * A Thread is an ordered list of Turns (one Q&A exchange each). Follow-ups
 * re-ask `ask_corpus` statelessly with prior turns folded into the query
 * (buildContextualQuery) — true cross-turn retrieval context is the daemon
 * follow-on (aaa-ask-multiturn-context). Threads persist per corpus in
 * localStorage so the history rail can resume them.
 */
import type { RecentEntry } from "./internals";

export type TurnStatus = "done" | "error";

/** A source dropped INTO the thread from a citation — a first-class, kept
 *  reference block (aaa-ask-citation-dropin). Persists with the thread so it
 *  survives resume, unlike the transient EntityPanel drawer. */
export interface DroppedSource {
  /** The source's content_id (section id or `sym-…`). */
  contentId: string;
  /** 1-based citation index it was opened from (for the [n] badge). Absent
   *  when the source was dropped from outside a numbered citation — e.g.
   *  Explore's "Ask about this symbol" (aaa-explore-integrated). */
  n?: number;
}

export interface Turn {
  id: string;
  query: string;
  status: TurnStatus;
  /** Turn kind. Absent on threads persisted before the source-drop feature —
   *  treat `undefined` as `"qa"` for back-compat. */
  kind?: "qa" | "source";
  /** Present when status === "done". */
  entry?: RecentEntry;
  /** Present when status === "error". */
  error?: string;
  /** Citation-checking flagged claims, if the verify phase reported any. */
  unsupported?: string[] | null;
  /** Present when kind === "source". */
  source?: DroppedSource;
}

/** Build a source-drop turn (a kept citation block in the thread). `n` is the
 *  citation index when dropped from a numbered citation; omit it for an
 *  out-of-band drop (e.g. Explore's "Ask about this symbol"). */
export function sourceTurn(contentId: string, n?: number): Turn {
  return {
    id: newId(),
    query: "",
    status: "done",
    kind: "source",
    source: n === undefined ? { contentId } : { contentId, n },
  };
}

export interface Thread {
  id: string;
  corpusId: string;
  turns: Turn[];
  createdAt: number;
  updatedAt: number;
}

/** A thread's display title is its first question. */
export function threadTitle(t: Thread): string {
  return t.turns[0]?.query ?? "New conversation";
}

export function newId(): string {
  try {
    return crypto.randomUUID();
  } catch {
    return `t-${Date.now()}-${Math.floor(Math.random() * 1e6)}`;
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-corpus localStorage persistence.

const THREADS_KEY = "ministr-ask-threads-v1";
export const THREADS_LIMIT = 30;

export function loadThreads(corpusId: string): Thread[] {
  try {
    const raw = localStorage.getItem(THREADS_KEY);
    if (!raw) return [];
    const all = JSON.parse(raw) as Record<string, Thread[]>;
    const list = all[corpusId];
    return Array.isArray(list) ? list : [];
  } catch {
    return [];
  }
}

export function saveThreads(corpusId: string, threads: Thread[]): void {
  try {
    const raw = localStorage.getItem(THREADS_KEY);
    const all = (raw ? JSON.parse(raw) : {}) as Record<string, Thread[]>;
    all[corpusId] = threads.slice(0, THREADS_LIMIT);
    localStorage.setItem(THREADS_KEY, JSON.stringify(all));
  } catch {
    /* localStorage unavailable — non-fatal */
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Follow-up context — fold prior turns into the query for a stateless re-ask.

const CTX_TURNS = 3; // cap how many prior turns we fold in
const CTX_ANSWER_CHARS = 400;

export function buildContextualQuery(
  priorTurns: Turn[],
  newQuery: string,
): string {
  const answered = priorTurns.filter((t) => t.status === "done" && t.entry);
  if (answered.length === 0) return newQuery;
  const lines = answered.slice(-CTX_TURNS).map((t) => {
    const a = (t.entry?.answer ?? "")
      .replace(/\s+/g, " ")
      .slice(0, CTX_ANSWER_CHARS);
    return `Q: ${t.query}\nA: ${a}`;
  });
  return `Earlier in this conversation:\n${lines.join("\n\n")}\n\nFollow-up question: ${newQuery}`;
}
