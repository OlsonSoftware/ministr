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

export interface Turn {
  id: string;
  query: string;
  status: TurnStatus;
  /** Present when status === "done". */
  entry?: RecentEntry;
  /** Present when status === "error". */
  error?: string;
  /** Citation-checking flagged claims, if the verify phase reported any. */
  unsupported?: string[] | null;
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
