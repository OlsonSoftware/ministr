import type { CorpusInfo, SessionDetail } from "../../lib/types";
import { sessionStatus } from "../../lib/sessions";
import { corpusLabel } from "../../lib/corpus";
import { StatusDot } from "../ui/status-dot";
import { cn } from "../../lib/utils";

/**
 * The ONE canonical compact renderer for a live session — the single component
 * the cross-cutting SessionLayer (and, in a follow-up, ProjectSessions + the
 * Cloud inspector) consume instead of each re-implementing a session row.
 * Pressure-aware dot via `sessionStatus`, agent identity, scope, turn, and
 * context utilization.
 */
export function SessionRow({
  session,
  corpora,
  onOpen,
}: {
  session: SessionDetail;
  corpora: CorpusInfo[];
  onOpen?: (session: SessionDetail) => void;
}) {
  const st = sessionStatus(session);
  const corpus = corpora.find((c) => c.id === session.corpus_id);
  const label = session.client_name || session.session_id.slice(0, 12);
  return (
    <button
      type="button"
      onClick={() => onOpen?.(session)}
      className={cn(
        "w-full flex items-center gap-2.5 px-2.5 py-2 rounded-md text-left cursor-pointer",
        "transition-colors duration-150 text-text-muted hover:bg-surface-overlay hover:text-text",
        "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent",
      )}
    >
      <StatusDot tone={st.tone} pulse="live" />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="font-mono text-xs font-medium text-text truncate">
            {label}
          </span>
          {session.parent_session_id && (
            <span className="shrink-0 rounded border border-border px-1 font-mono text-[10px] uppercase tracking-[0.08em] text-text-dim">
              sub
            </span>
          )}
        </div>
        <div className="font-mono text-mono-mini text-text-dim truncate">
          {corpus ? corpusLabel(corpus) : session.corpus_id} · turn{" "}
          {session.current_turn}
        </div>
      </div>
      <span className="shrink-0 font-mono text-mono-mini tabular-nums text-text-dim">
        {st.pct}%
      </span>
    </button>
  );
}
