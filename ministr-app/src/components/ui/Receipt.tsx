import { TrustMark } from "./TrustMark";

/**
 * Receipt — one recorded event, restated in plain words (DESIGN.md §7).
 * Wins and heads-ups share IDENTICAL typography and size (§2.3 —
 * equal-weight honesty); only the mark differs.
 */
export function Receipt({
  time,
  sentence,
  kind,
}: {
  /** Clock time of the recorded event, e.g. "10:43". */
  time: string;
  /** The 1:1 plain-words restatement of the event. */
  sentence: string;
  /** Optional verdict: a win (✓) or a heads-up (⚠). Omit for neutral. */
  kind?: "win" | "headsup";
}) {
  return (
    <div className="flex items-baseline gap-3 px-2 py-1.5 text-sm">
      <span className="shrink-0 font-mono text-xs text-dim">{time}</span>
      {kind ? <TrustMark state={kind === "win" ? "ok" : "stale"} /> : null}
      <p className="text-ink">{sentence}</p>
    </div>
  );
}
