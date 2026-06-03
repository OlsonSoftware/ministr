import { memo } from "react";
import { Zap, Gauge, Copy, TrendingDown, AlertTriangle } from "lucide-react";
import type { CorpusInfo, SessionDetail } from "../../lib/types";
import { corpusLabelById } from "../../lib/corpus";
import { toneBgClass, toneTextClass } from "../../lib/status";
import { clampPct, statusLabel, utilizationTone } from "../../lib/sessions";
import { cn } from "../../lib/utils";
import { focusRing } from "../../lib/ui-tokens";
import { formatTokens } from "../../lib/format";
import { MetricTile } from "./metric-tile";
import { BudgetBar } from "./budget-bar";
import { StatusDot } from "./status-dot";

interface TurnBlockProps {
  session: SessionDetail;
  /** Optional corpora list so the footer can render the corpus's
   *  human-readable name. */
  corpora?: readonly CorpusInfo[] | null;
  /** True if this session just ticked a new turn (drives the flash). */
  fresh?: boolean;
  onClick?: () => void;
  className?: string;
}

/**
 * Session card. Pressure colour is derived from utilization (via
 * `lib/sessions`), not the raw enum — so it can't regress to grey. A
 * persistent left identity edge carries the status colour at a glance;
 * the budget bar is the framed `BudgetBar`, not a 1.5px sliver.
 * `React.memo`'d: the shared store hands a stable session ref when
 * unchanged, so untouched cards skip re-render under poll.
 */
function TurnBlockImpl({
  session,
  corpora,
  fresh,
  onClick,
  className,
}: TurnBlockProps) {
  const tone = utilizationTone(session.utilization);
  const utilPct = clampPct(session.utilization * 100);
  const sessionShort = session.session_id.slice(0, 8);
  const critical = tone === "danger";

  return (
    <div
      onClick={onClick}
      // §9 WCAG 2.1.1/4.1.2 — a clickable card must be keyboard-operable.
      role={onClick ? "button" : undefined}
      tabIndex={onClick ? 0 : undefined}
      onKeyDown={
        onClick
          ? (e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onClick();
              }
            }
          : undefined
      }
      className={cn(
        // overflow-hidden so the absolute status edge clips to the radius
        "group relative overflow-hidden rounded-lg border border-border bg-surface p-3 pl-3.5",
        "transition-[border-color,box-shadow,transform] duration-150 ease-out",
        onClick &&
          cn(
            "cursor-pointer hover:-translate-y-0.5 hover:border-border-hover hover:shadow-md",
            focusRing,
          ),
        fresh && "ministr-pulse",
        className,
      )}
    >
      {/* Persistent identity edge — status colour at a glance. */}
      <span
        className={cn(
          "absolute left-0 top-0 bottom-0 w-0.5",
          toneBgClass(tone),
        )}
        aria-hidden="true"
      />

      {/* Header row: session glyph + id + turn + status */}
      <div className="flex items-center gap-2 mb-2">
        <StatusDot tone={tone} pulse={fresh ? "live" : "off"} size="md" />
        <span className="font-mono text-mono-mini text-text-muted truncate min-w-0">
          {sessionShort}
        </span>
        <span className="font-mono text-mono-mini text-text-dim shrink-0">·</span>
        {/* Turn count is core info — never truncate it. */}
        <span className="font-mono text-mono-mini text-text shrink-0">
          turn {session.current_turn}
        </span>
        {session.parent_session_id && (
          <span
            className="inline-flex items-center gap-1 border border-border-soft bg-surface-overlay px-1.5 py-0 text-xs font-mono text-text-muted"
            title={`Subagent of ${session.parent_session_id.slice(0, 8)}`}
          >
            <span aria-hidden="true">↳</span>
            sub
          </span>
        )}
        {session.client_name && (
          <span className="font-mono text-xs text-text-dim truncate max-w-[120px]">
            {session.client_name}
          </span>
        )}
        <div className="flex-1" />
        <span
          className={cn(
            "font-mono text-mono-mini font-bold uppercase tracking-[0.08em]",
            toneTextClass(tone),
          )}
        >
          {statusLabel(tone)}
        </span>
      </div>

      {/* Metrics row */}
      <div className="grid grid-cols-4 gap-2 text-mono-mini">
        <MetricTile
          variant="compact"
          icon={Gauge}
          value={`${utilPct}%`}
          label="budget"
        />
        <MetricTile
          variant="compact"
          icon={Zap}
          value={formatTokens(session.tokens_used)}
          label="tokens"
        />
        <MetricTile
          variant="compact"
          icon={TrendingDown}
          value={formatTokens(session.total_tokens_saved)}
          label="saved"
          tone="success"
        />
        <MetricTile
          variant="compact"
          icon={Copy}
          value={session.dedup_hits.toLocaleString()}
          label="repeats"
          tone="accent"
        />
      </div>

      {/* Budget bar — framed, colour from utilization, % at the edge. */}
      <div className="mt-2.5">
        <BudgetBar utilization={session.utilization} size="card" showValue />
      </div>

      {/* Footer: project name */}
      <div className="mt-2 flex items-center gap-1.5 text-xs font-mono text-text-dim truncate">
        <span className="uppercase tracking-[0.08em]">project</span>
        <span className="text-text-muted truncate">
          {corpusLabelById(corpora, session.corpus_id)}
        </span>
        {critical && (
          <span className="inline-flex items-center gap-1 ml-auto text-danger uppercase tracking-[0.08em] font-semibold">
            <AlertTriangle className="h-3 w-3" strokeWidth={2.5} />
            evicting
          </span>
        )}
      </div>
    </div>
  );
}

export const TurnBlock = memo(TurnBlockImpl);
