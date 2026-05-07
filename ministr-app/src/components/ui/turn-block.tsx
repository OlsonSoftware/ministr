import {
  Zap,
  Gauge,
  Copy,
  TrendingDown,
  AlertTriangle,
} from "lucide-react";
import type { CorpusInfo, SessionDetail } from "../../lib/types";
import { corpusLabelById } from "../../lib/corpus";
import { pressureTone, toneTextClass } from "../../lib/status";
import { cn } from "../../lib/utils";
import { formatTokens } from "../../lib/format";
import { MetricTile } from "./metric-tile";
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

export function TurnBlock({ session, corpora, fresh, onClick, className }: TurnBlockProps) {
  const tone = pressureTone(session.pressure_level);
  const pressureColor = toneTextClass(tone);
  const utilPct = (session.utilization * 100).toFixed(0);
  const sessionShort = session.session_id.slice(0, 8);

  return (
    <div
      onClick={onClick}
      className={cn(
        "group relative border border-border-soft bg-surface p-3 transition-none",
        onClick &&
          "cursor-pointer hover:-translate-x-[2px] hover:-translate-y-[2px] hover:shadow-md",
        fresh && "ministr-flash",
        className,
      )}
    >
      {/* Header row: session glyph + id + turn + pressure */}
      <div className="flex items-center gap-2 mb-2">
        <StatusDot tone={tone} pulse={fresh ? "live" : "off"} size="md" />
        <span className="font-mono text-mono-mini text-text-muted truncate">
          {sessionShort}
        </span>
        <span className="font-mono text-mono-mini text-text-dim">·</span>
        <span className="font-mono text-mono-mini text-text truncate">
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
        <span className={cn("font-mono text-mono-mini font-bold uppercase tracking-[0.05em]", pressureColor)}>
          {session.pressure_level}
        </span>
      </div>

      {/* Metrics row */}
      <div className="grid grid-cols-4 gap-2 text-mono-mini">
        <MetricTile variant="compact" icon={Gauge} value={`${utilPct}%`} label="budget" />
        <MetricTile variant="compact" icon={Zap} value={formatTokens(session.tokens_used)} label="tokens" />
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
          value={session.dedup_hits.toString()}
          label="dedup"
          tone="accent"
        />
      </div>

      {/* Budget bar sliver — sharp, no rounded ends. */}
      <div className="mt-2.5 h-1.5 border border-border-soft bg-surface-overlay overflow-hidden">
        <div
          className={cn(
            "h-full transition-none",
            session.pressure_level === "critical" && "bg-danger",
            session.pressure_level === "high" && "bg-warning",
            (session.pressure_level === "medium"
              || session.pressure_level === "low"
              || session.pressure_level === "none") && "bg-accent",
          )}
          style={{ width: `${utilPct}%` }}
        />
      </div>

      {/* Footer: corpus name */}
      <div className="mt-2 flex items-center gap-1.5 text-xs font-mono text-text-dim truncate">
        <span className="uppercase tracking-[0.05em]">corpus</span>
        <span className="text-text-muted truncate">
          {corpusLabelById(corpora, session.corpus_id)}
        </span>
        {session.pressure_level === "critical" && (
          <span className="inline-flex items-center gap-1 ml-auto text-danger uppercase tracking-[0.05em] font-semibold">
            <AlertTriangle className="h-3 w-3" strokeWidth={2.5}/>
            evict
          </span>
        )}
      </div>
    </div>
  );
}
