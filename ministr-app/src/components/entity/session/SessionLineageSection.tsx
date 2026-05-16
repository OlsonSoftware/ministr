import type { SessionDetail } from "../../../lib/types";
import { ChevronRight } from "lucide-react";
import {
  clampPct,
  pressureVerdict,
  utilizationTone,
} from "../../../lib/sessions";
import { toneTextClass } from "../../../lib/status";
import { cn } from "../../../lib/utils";
import { EntitySection } from "../EntitySection";
import { StatusDot } from "../../ui/status-dot";

interface Props {
  chapter: number;
  session: SessionDetail;
  parent: SessionDetail | null;
  children: SessionDetail[];
  onOpen: (s: SessionDetail) => void;
}

function LineageRow({
  s,
  role,
  depth,
  current,
  onOpen,
}: {
  s: SessionDetail;
  role: string;
  depth: number;
  current?: boolean;
  onOpen?: (s: SessionDetail) => void;
}) {
  const tone = utilizationTone(s.utilization);
  return (
    <button
      type="button"
      disabled={current || !onOpen}
      onClick={() => onOpen?.(s)}
      style={{ paddingLeft: depth * 16 + 12 }}
      className={cn(
        "group w-full text-left flex items-center gap-2 pr-3 py-2 transition-none",
        "border-b border-border-soft last:border-b-0",
        current
          ? "border-l-2 border-accent bg-surface-overlay cursor-default"
          : "cursor-pointer hover:bg-surface-overlay focus-visible:bg-surface-overlay",
      )}
    >
      <StatusDot tone={tone} />
      <span
        className={cn(
          "font-mono text-sm tabular-nums truncate",
          current ? "font-bold text-text" : "font-semibold text-text",
        )}
      >
        {s.session_id.slice(0, 8)}
      </span>
      <span className="font-mono text-mono-mini uppercase tracking-[0.05em] text-text-dim shrink-0">
        {role}
      </span>
      <div className="flex-1" />
      <span className="font-mono text-mono-mini tabular-nums text-text-dim shrink-0">
        turn {s.current_turn}
      </span>
      <span
        className={cn(
          "font-mono text-mono-mini uppercase tracking-[0.05em] shrink-0",
          toneTextClass(tone),
        )}
      >
        {pressureVerdict(s.pressure_level).word} · {clampPct(s.utilization * 100)}%
      </span>
      {!current && onOpen ? (
        <ChevronRight
          className="h-3.5 w-3.5 shrink-0 text-text-dim group-hover:text-text"
          strokeWidth={2}
        />
      ) : (
        <span className="w-3.5 shrink-0" />
      )}
    </button>
  );
}

/**
 * §Lineage — where this session sits in the agent tree. Rendered only
 * when there is a parent or children (depth-1; agents spawn subagents one
 * level via the Task tool). Built client-side from the shared session
 * list, so every row is a live click-through.
 */
export function SessionLineageSection({
  chapter,
  session,
  parent,
  children,
  onOpen,
}: Props) {
  return (
    <EntitySection
      chapter={chapter}
      title="Lineage"
      meta={1 + (parent ? 1 : 0) + children.length}
    >
      {parent && (
        <LineageRow s={parent} role="parent" depth={0} onOpen={onOpen} />
      )}
      <LineageRow
        s={session}
        role="this session"
        depth={parent ? 1 : 0}
        current
      />
      {children.map((c) => (
        <LineageRow
          key={c.session_id}
          s={c}
          role="subagent"
          depth={parent ? 2 : 1}
          onOpen={onOpen}
        />
      ))}
    </EntitySection>
  );
}
