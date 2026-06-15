import type { ReactNode } from "react";
import { TrustMark } from "./TrustMark";
import type { TrustState } from "./trust";

/**
 * TreeRow — one file's truth in the Mirror tree (DESIGN.md §7).
 * Mono name (it's a path), mark, optional plain-words note, optional
 * action. Indentation is structural (level), not decorative.
 *
 * `disclosure` makes the row announce its interactive role (the row is
 * wrapped in a real <button> by TreeBranch): a leading caret for an
 * expandable directory (▸ closed / ▾ open) and a quiet trailing chevron
 * for a file that opens a drill-in (revealed on hover — calm until you
 * reach for it). Neutral furniture only; no second hue.
 */
export function TreeRow({
  name,
  state,
  note,
  level = 0,
  action,
  disclosure,
}: {
  name: string;
  state: TrustState;
  note?: string;
  level?: number;
  action?: ReactNode;
  disclosure?: "expandable" | "expanded" | "navigates";
}) {
  const isDir = disclosure === "expandable" || disclosure === "expanded";
  return (
    <div
      className="group/row flex items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-sunken"
      style={{ paddingLeft: `${0.5 + level * 1.25}rem` }}
    >
      {isDir ? (
        <span aria-hidden className="w-3 shrink-0 text-center text-xs text-dim">
          {disclosure === "expanded" ? "▾" : "▸"}
        </span>
      ) : null}
      <TrustMark state={state} />
      <span className="font-mono text-ink">{name}</span>
      {note ? <span className="truncate text-dim">{note}</span> : null}
      {action ? <span className="ml-auto shrink-0">{action}</span> : null}
      {disclosure === "navigates" && !action ? (
        <span
          aria-hidden
          className="ml-auto shrink-0 text-dim opacity-0 transition-opacity group-hover/row:opacity-100"
        >
          ›
        </span>
      ) : null}
    </div>
  );
}
