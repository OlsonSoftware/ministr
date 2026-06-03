/**
 * SessionCard — the ONE rich session renderer (aaa-session-renderer-dedup).
 *
 * Collapses the two former rich cards (SessionsSurface's board card +
 * ProjectSessions' TurnBlock) into a single component with an
 * `interaction` prop:
 *
 *   - `"expand"`  — the Activity board: the header toggles an in-place
 *                   economics dashboard (SessionExpanded); supports lineage
 *                   nesting via `child`. (Inspector is one click away inside.)
 *   - `"inspect"` — the Projects/Tend slice: the whole card opens the deep
 *                   EntityPanel inspector on click.
 *
 * The compact `⚡` SessionRow is a DIFFERENT density and stays separate.
 *
 * Built from atoms (BudgetRing, BudgetBar, Sparkline, StatusDot, MetricTile)
 * — a fresh composition, not a re-skin of either old card. `memo`'d so the
 * shared session store's stable refs skip re-render under poll.
 */
import { memo } from "react";
import {
  AlertTriangle,
  ChevronDown,
  Copy,
  ExternalLink,
  Flame,
  Gauge,
  Layers,
  Scissors,
  Shrink,
  TrendingDown,
  Zap,
} from "lucide-react";
import { motion } from "motion/react";

import type { CorpusInfo, SessionDetail } from "../../lib/types";
import { corpusLabel, corpusLabelById } from "../../lib/corpus";
import { formatTokens } from "../../lib/format";
import { toneBgClass, toneTextClass } from "../../lib/status";
import {
  burnRate,
  clampPct,
  deriveVitals,
  pressureFromUtil,
  pressureVerdict,
  projectCritical,
  utilizationTone,
  type SessionSample,
} from "../../lib/sessions";
import { cn } from "../../lib/utils";
import { focusRing } from "../../lib/ui-tokens";
import { BudgetBar } from "./budget-bar";
import { BudgetRing } from "./budget-ring";
import { MetricTile } from "./metric-tile";
import { Sparkline } from "./sparkline";
import { StatusDot } from "./status-dot";

export type SessionInteraction = "expand" | "inspect";

export interface SessionCardProps {
  session: SessionDetail;
  /** How the card behaves: board (expand-in-place, the default) vs slice
   *  (open inspector on whole-card click). */
  interaction?: SessionInteraction;
  /** Single corpus (board passes the resolved one). */
  corpus?: CorpusInfo;
  /** Corpus list for label fallback (Projects passes [corpus]). */
  corpora?: readonly CorpusInfo[] | null;
  /** Token-usage samples — drives the sparkline + projection (board has them;
   *  the Projects slice doesn't, and falls back to a budget bar). */
  samples?: readonly SessionSample[];
  /** True if this session just ticked — drives the flash. */
  fresh?: boolean;
  /** expand mode: whether the in-place dashboard is open. */
  expanded?: boolean;
  /** expand mode: toggle the dashboard. */
  onToggle?: () => void;
  /** expand mode: "Open full inspector"; inspect mode: the whole-card click. */
  onOpenInspector?: () => void;
  /** expand mode: a nested subagent card (smaller ring). */
  child?: boolean;
}

function SessionCardImpl({
  session: s,
  interaction = "expand",
  corpus,
  corpora,
  samples = [],
  fresh = false,
  expanded = false,
  onToggle,
  onOpenInspector,
  child = false,
}: SessionCardProps) {
  const tone = utilizationTone(s.utilization);
  const pct = clampPct(s.utilization * 100);
  const verdict = pressureVerdict(s.pressure_level);
  const critical = tone === "danger";
  const series = samples.map((x) => x.tokensUsed);
  const hasTrend = series.length > 1;
  const proj = projectCritical(s, samples);
  const ringSize = child ? 44 : 56;
  const label = corpus
    ? corpusLabel(corpus)
    : corpusLabelById(corpora ?? null, s.corpus_id);
  const expandable = interaction === "expand";

  const header = (
    <div className="flex items-start gap-3">
      <BudgetRing
        utilization={s.utilization}
        pressure={pressureFromUtil(s.utilization)}
        size={ringSize}
        stroke={6}
      >
        <span className="font-mono text-sm font-semibold tabular-nums text-text leading-none">
          {pct}
          <span className="text-mono-micro text-text-dim">%</span>
        </span>
      </BudgetRing>

      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <StatusDot tone={tone} pulse="live" />
          <span className="font-mono text-sm font-semibold text-text truncate">
            {s.session_id.slice(0, 12)}
          </span>
          {s.parent_session_id && (
            <span
              className="inline-flex items-center gap-0.5 border border-border-soft bg-surface-overlay px-1 rounded text-mono-micro font-mono text-text-muted shrink-0"
              title={`Subagent of ${s.parent_session_id.slice(0, 8)}`}
            >
              <span aria-hidden>↳</span>sub
            </span>
          )}
          <span className="flex-1" />
          {expandable ? (
            <ChevronDown
              className={cn(
                "h-4 w-4 text-text-dim shrink-0 transition-transform duration-200",
                expanded && "rotate-180",
              )}
              strokeWidth={2}
            />
          ) : (
            <span
              className={cn(
                "font-mono text-mono-mini font-bold uppercase tracking-[0.08em] shrink-0",
                toneTextClass(tone),
              )}
            >
              {verdict.word}
            </span>
          )}
        </div>
        {expandable && (
          <p
            className={cn(
              "font-mono text-mono-mini uppercase tracking-[0.06em] mt-1",
              toneTextClass(tone),
            )}
          >
            {verdict.word}
          </p>
        )}
        <p className="font-mono text-mono-mini text-text-dim mt-0.5 truncate">
          {label}
          {s.client_name ? ` · ${s.client_name}` : ""}
        </p>
      </div>
    </div>
  );

  const metrics = (
    <div className="grid grid-cols-4 gap-2 text-mono-mini">
      <MetricTile variant="compact" icon={Gauge} value={`${pct}%`} label="budget" />
      <MetricTile
        variant="compact"
        icon={Zap}
        value={formatTokens(s.tokens_used)}
        label="tokens"
      />
      <MetricTile
        variant="compact"
        icon={TrendingDown}
        value={formatTokens(s.total_tokens_saved)}
        label="saved"
        tone="success"
      />
      <MetricTile
        variant="compact"
        icon={Copy}
        value={s.dedup_hits.toLocaleString()}
        label="repeats"
        tone="accent"
      />
    </div>
  );

  const trend = hasTrend ? (
    <Sparkline
      data={series}
      smooth
      tone={tone}
      height={32}
      ariaLabel={`Token usage trend for ${s.session_id}`}
    />
  ) : (
    <BudgetBar utilization={s.utilization} size="card" showValue />
  );

  const footer = (
    <div className="flex items-center justify-between gap-2 font-mono text-mono-mini text-text-dim">
      <span>turn {s.current_turn}</span>
      {expandable && proj?.turns != null ? (
        <span className={toneTextClass(tone)}>
          ≈ {proj.turns} turn{proj.turns === 1 ? "" : "s"} to limit
        </span>
      ) : critical ? (
        <span className="inline-flex items-center gap-1 text-danger uppercase tracking-[0.08em] font-semibold">
          <AlertTriangle className="h-3 w-3" strokeWidth={2.5} />
          evicting
        </span>
      ) : (
        <span className="tabular-nums">
          {formatTokens(s.tokens_used)} / {formatTokens(s.tokens_remaining)} free
        </span>
      )}
    </div>
  );

  const body = (
    <div className="flex flex-col gap-2.5">
      {header}
      {metrics}
      <div className="-mx-0.5">{trend}</div>
      {footer}
    </div>
  );

  // ── inspect mode: the whole card opens the inspector. ──────────────────
  if (!expandable) {
    return (
      <motion.div
        layout
        onClick={onOpenInspector}
        role={onOpenInspector ? "button" : undefined}
        tabIndex={onOpenInspector ? 0 : undefined}
        onKeyDown={
          onOpenInspector
            ? (e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onOpenInspector();
                }
              }
            : undefined
        }
        whileHover={{ y: -2 }}
        className={cn(
          "group relative overflow-hidden rounded-lg border bg-surface p-3.5",
          "transition-[border-color,box-shadow] duration-150 ease-out",
          critical ? "border-danger/50" : "border-border hover:border-border-hover",
          onOpenInspector && cn("cursor-pointer hover:shadow-md", focusRing),
          fresh && "ministr-pulse",
        )}
      >
        <span
          className={cn("absolute left-0 top-0 bottom-0 w-0.5", toneBgClass(tone))}
          aria-hidden
        />
        {body}
      </motion.div>
    );
  }

  // ── expand mode: the header toggles an in-place dashboard. ─────────────
  return (
    <motion.div
      layout
      whileHover={expanded ? undefined : { y: -2 }}
      className={cn(
        "group rounded-lg border bg-surface shadow-xs",
        "transition-[border-color,box-shadow] duration-200",
        critical ? "border-danger/50" : "border-border hover:border-border-hover",
        expanded && "shadow-md",
        fresh && "ministr-pulse",
      )}
    >
      <button
        onClick={onToggle}
        aria-expanded={expanded}
        className={cn("w-full text-left p-3.5 cursor-pointer rounded-lg", focusRing)}
      >
        {body}
      </button>
      {expanded && (
        <SessionExpanded
          session={s}
          samples={samples}
          onOpenInspector={onOpenInspector}
        />
      )}
    </motion.div>
  );
}

export const SessionCard = memo(SessionCardImpl);

/** The in-place per-session dashboard revealed when a board card expands —
 *  economics grid + burn/projection + a larger trend, no panel switch. */
function SessionExpanded({
  session,
  samples,
  onOpenInspector,
}: {
  session: SessionDetail;
  samples: readonly SessionSample[];
  onOpenInspector?: () => void;
}) {
  const v = deriveVitals(session);
  if (!v) return null;
  const burn = burnRate(samples);
  const proj = projectCritical(session, samples);
  const series = samples.map((s) => s.tokensUsed);

  return (
    <div className="border-t border-border-soft px-3.5 py-3 flex flex-col gap-3">
      <div className="grid grid-cols-2 gap-px rounded-lg overflow-hidden border border-border-soft bg-border-soft">
        <MetricTile
          variant="cell"
          icon={Zap}
          label="saved"
          value={formatTokens(v.tokensSaved)}
          tone="success"
          className="bg-surface"
        />
        <MetricTile
          variant="cell"
          icon={Layers}
          label="dedup hits"
          value={String(v.dedupHits)}
          className="bg-surface"
        />
        <MetricTile
          variant="cell"
          icon={Scissors}
          label="evictions"
          value={String(v.evictions)}
          className="bg-surface"
        />
        <MetricTile
          variant="cell"
          icon={Shrink}
          label="compress"
          value={String(v.compressions)}
          className="bg-surface"
        />
      </div>

      <div className="flex items-center justify-between font-mono text-mono-mini text-text-dim">
        <span className="inline-flex items-center gap-1">
          <Flame className="h-3 w-3" strokeWidth={2} aria-hidden />
          {burn.tokensPerTurn != null
            ? `${Math.round(burn.tokensPerTurn)} tok/turn`
            : "stable burn"}
        </span>
        <span className={cn(proj?.turns != null && toneTextClass(v.tone))}>
          {proj?.turns != null
            ? `≈ ${proj.turns} turn${proj.turns === 1 ? "" : "s"} to limit`
            : "not trending up"}
        </span>
      </div>

      {series.length > 1 && (
        <Sparkline
          data={series}
          smooth
          tone={v.tone}
          height={44}
          ariaLabel={`Token usage trend for ${session.session_id}`}
        />
      )}

      {onOpenInspector && (
        <button
          onClick={onOpenInspector}
          className="self-start inline-flex items-center gap-1 font-mono text-mono-mini uppercase tracking-[0.08em] text-text-muted hover:text-text cursor-pointer transition-colors duration-150"
        >
          Open full inspector
          <ExternalLink className="h-3 w-3" strokeWidth={2} />
        </button>
      )}
    </div>
  );
}

/** Loading placeholder mirroring a SessionCard's layout (ring + title +
 *  trend + footer) so the board previews its structure while the first poll
 *  lands — no blank gap, no layout jump. */
export function SessionCardSkeleton() {
  return (
    <div
      className="rounded-lg border border-border bg-surface p-3.5 shadow-xs"
      aria-hidden
    >
      <div className="flex items-start gap-3">
        <div
          className="h-14 w-14 shrink-0 ministr-skeleton"
          style={{ borderRadius: "9999px" }}
        />
        <div className="min-w-0 flex-1 space-y-2 pt-1">
          <div className="h-3 w-2/3 ministr-skeleton" />
          <div className="h-2 w-1/3 ministr-skeleton" />
          <div className="h-2 w-1/2 ministr-skeleton" />
        </div>
      </div>
      <div className="mt-3 h-8 ministr-skeleton" />
      <div className="mt-2 flex items-center justify-between">
        <div className="h-2 w-12 ministr-skeleton" />
        <div className="h-2 w-20 ministr-skeleton" />
      </div>
    </div>
  );
}
