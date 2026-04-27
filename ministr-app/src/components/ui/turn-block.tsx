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
import { MetricTile } from "./metric-tile";
import { StatusDot } from "./status-dot";

interface TurnBlockProps {
  session: SessionDetail;
  /** Optional corpora list so the footer can render the corpus's
   *  human-readable name instead of its raw `multi-…` id. */
  corpora?: readonly CorpusInfo[] | null;
  /** True if this session just ticked a new turn (drives the flash). */
  fresh?: boolean;
  onClick?: () => void;
  className?: string;
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toString();
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
        "group relative rounded-xl border border-border/60 bg-surface-raised/60 p-3 transition-all duration-150",
        onClick && "cursor-pointer hover:border-border-hover",
        fresh && "ministr-flash",
        className,
      )}
    >
      {/* Header row: session glyph + id + turn + pressure */}
      <div className="flex items-center gap-2 mb-2">
        <StatusDot tone={tone} pulse={fresh ? "live" : "off"} size="md" />
        <span className="font-mono text-[11px] text-text-muted truncate">
          {sessionShort}
        </span>
        <span className="font-mono text-[11px] text-text-dim">·</span>
        <span className="font-mono text-[11px] text-text truncate">
          turn {session.current_turn}
        </span>
        <div className="flex-1" />
        <span className={cn("font-mono text-[11px] font-semibold uppercase tracking-wider", pressureColor)}>
          {session.pressure_level}
        </span>
      </div>

      {/* Metrics row: budget gauge inline + delivered + saved + dedup */}
      <div className="grid grid-cols-4 gap-2 text-[11px]">
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

      {/* Budget bar sliver */}
      <div className="mt-2.5 h-1 rounded-full bg-surface-overlay/80 overflow-hidden">
        <div
          className={cn(
            "h-full rounded-full transition-all duration-400",
            session.pressure_level === "critical" && "bg-danger",
            session.pressure_level === "high" && "bg-warning",
            session.pressure_level === "medium" && "bg-accent",
            (session.pressure_level === "low" || session.pressure_level === "none") &&
              "bg-gradient-to-r from-accent to-[color-mix(in_srgb,var(--color-accent)_60%,#c4b5fd)]",
          )}
          style={{ width: `${utilPct}%` }}
        />
      </div>

      {/* Footer: corpus name (resolved from id via the parent's corpora list). */}
      <div className="mt-2 flex items-center gap-1.5 text-[10px] font-mono text-text-dim truncate">
        <span>corpus</span>
        <span className="text-text-muted truncate">
          {corpusLabelById(corpora, session.corpus_id)}
        </span>
        {session.pressure_level === "critical" && (
          <span className="inline-flex items-center gap-1 ml-auto text-danger">
            <AlertTriangle className="h-3 w-3" />
            evict
          </span>
        )}
      </div>
    </div>
  );
}

