import type { ReactNode } from "react";
import { TrustMark } from "./TrustMark";
import type { TrustState } from "./trust";

/**
 * TreeRow — one file's truth in the Mirror tree (DESIGN.md §7).
 * Mono name (it's a path), mark, optional plain-words note, optional
 * action. Indentation is structural (level), not decorative.
 */
export function TreeRow({
  name,
  state,
  note,
  level = 0,
  action,
}: {
  name: string;
  state: TrustState;
  note?: string;
  level?: number;
  action?: ReactNode;
}) {
  return (
    <div
      className="flex items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-sunken"
      style={{ paddingLeft: `${0.5 + level * 1.25}rem` }}
    >
      <TrustMark state={state} />
      <span className="font-mono text-ink">{name}</span>
      {note ? <span className="truncate text-dim">{note}</span> : null}
      {action ? <span className="ml-auto shrink-0">{action}</span> : null}
    </div>
  );
}
