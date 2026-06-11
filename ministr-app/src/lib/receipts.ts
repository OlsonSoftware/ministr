import type { ActivityEvent, OutcomeEventInfo, OutcomesResponse } from "./ipc";

/**
 * Receipts derivation (UX-BLUEPRINT §3.3, DESIGN §2.3/§2.5): every
 * receipt restates EXACTLY ONE recorded event in plain words. The
 * outcome wording is the JOIN FACT ("you changed X — your AI had read
 * it"), never a temporal claim the data can't support.
 */

export interface ReceiptLine {
  /** Unix ms of the underlying event (sort key). */
  ts: number;
  /** Plain-words restatement, 1:1 with the event. */
  sentence: string;
  /** Verdict mark: win (✓) / heads-up (⚠) / none (neutral). */
  kind?: "win" | "headsup";
}

const base = (p: string) => p.split("/").pop() ?? p;

/** One tool call → one sentence. Unknown tools restate honestly. */
export function activitySentence(e: ActivityEvent): string {
  const s = e.summary;
  switch (e.tool) {
    case "ministr_survey":
      return `your AI searched “${s}”`;
    case "ministr_read":
      return `your AI read ${base(s)}`;
    case "ministr_definition":
      return `your AI looked up ${s.split("::").pop() ?? s}`;
    case "ministr_references":
      return `your AI traced who uses ${s.split("::").pop() ?? s}`;
    case "ministr_symbols":
      return `your AI scanned for symbols matching “${s}”`;
    case "ministr_extract":
      return `your AI pulled the key facts from ${base(s)}`;
    case "ministr_toc":
      return "your AI skimmed the project layout";
    case "ministr_bridge":
      return "your AI followed a cross-language link";
    default:
      return `your AI used ${e.tool.replace("ministr_", "")}${s ? ` (${s})` : ""}`;
  }
}

/** One outcome join → one sentence, the join fact only. */
export function outcomeSentence(o: OutcomeEventInfo): ReceiptLine {
  const name = base(o.path);
  if (o.first_touch) {
    return {
      ts: o.edited_at_ms,
      kind: "win",
      sentence: `you changed ${name} — the first file your AI read`,
    };
  }
  return {
    ts: o.edited_at_ms,
    kind: "headsup",
    sentence: `you changed ${name} — your AI had read it (file #${o.read_rank} it looked at)`,
  };
}

/** Merge tool-call + outcome receipts, newest first. */
export function buildFeed(
  activity: ActivityEvent[],
  outcomes: OutcomeEventInfo[],
): ReceiptLine[] {
  const lines: ReceiptLine[] = [
    ...activity.map((e) => ({
      ts: e.timestamp_ms,
      sentence: activitySentence(e),
    })),
    ...outcomes.map(outcomeSentence),
  ];
  return lines.sort((a, b) => b.ts - a.ts);
}

/** Counts-only aggregate line (DESIGN §2.5 — no synthesis). */
export function aggregate(
  activity: ActivityEvent[],
  outcomes: OutcomesResponse,
): string {
  const searches = activity.filter((e) => e.tool === "ministr_survey").length;
  const reads = activity.filter((e) => e.tool === "ministr_read").length;
  const fromMemory = activity.filter((e) => e.cache_hit).length;
  const joins = outcomes.events.length;
  const firstTouch = outcomes.events.filter((e) => e.first_touch).length;

  const parts = [
    `${searches} search${searches === 1 ? "" : "es"}`,
    `${reads} read${reads === 1 ? "" : "s"}`,
  ];
  if (fromMemory > 0) parts.push(`${fromMemory} answered from memory`);
  if (joins > 0)
    parts.push(`${joins} file${joins === 1 ? "" : "s"} your AI read got edited (${firstTouch} on its first read)`);
  return parts.join(" · ");
}

export function clock(ts: number): string {
  return new Date(ts).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}
