/**
 * SessionsSurface — mission control: a live board of every agent session
 * consuming the cache. Cards EXPAND IN PLACE to a per-session economics
 * dashboard (no panel/route switch); subagents NEST under their parent as a
 * lineage tree; the board AUTO-SORTS by pressure so a critical session
 * surfaces itself. The deep EntityPanel inspector remains one click away from
 * the expanded view. Reuses the shared `useSessions` store + `lib/sessions`
 * derivations — no own fetch.
 */
import { useMemo, useState } from "react";
import {
  ChevronDown,
  ExternalLink,
  Flame,
  Layers,
  Scissors,
  Shrink,
  Users,
  Zap,
} from "lucide-react";
import { motion } from "motion/react";

import type { CorpusInfo, DaemonStatus, SessionDetail } from "../../lib/types";
import { corpusLabel } from "../../lib/corpus";
import { formatTokens } from "../../lib/format";
import { toneTextClass } from "../../lib/status";
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
import { buildLineageGroups, type LineageGroup } from "../../lib/session-board";
import { listContainer, listItem } from "../../lib/motion";
import { cn } from "../../lib/utils";
import { useEntityPanel } from "../../hooks/useEntityPanel";
import { useSessions } from "../../hooks/useSessions";

import { BudgetRing } from "../ui/budget-ring";
import { EmptyState } from "../ui/empty-state";
import { MetricTile } from "../ui/metric-tile";
import { NumberTicker } from "../ui/number-ticker";
import { Sparkline } from "../ui/sparkline";
import { StatusDot } from "../ui/status-dot";
import { H1 } from "../ui/heading";
import { AdaptiveSurface } from "../ui/adaptive-surface";

const CONNECT_CMD = "npx @modelcontextprotocol/inspector ministr stdio";

export function SessionsSurface({
  status,
}: {
  status: DaemonStatus;
  activeCorpusId: string | null;
}) {
  const { sessions, byId, samples, freshIds, loaded } = useSessions();
  const { openEntity } = useEntityPanel();
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const corpora = status.corpora;
  const corpusById = useMemo(
    () => new Map(corpora.map((c) => [c.id, c])),
    [corpora],
  );

  const agg = useMemo(() => {
    const used = sessions.reduce((a, s) => a + s.tokens_used, 0);
    const cap = sessions.reduce(
      (a, s) => a + s.tokens_used + s.tokens_remaining,
      0,
    );
    return {
      count: sessions.length,
      util: cap > 0 ? used / cap : 0,
      saved: sessions.reduce((a, s) => a + s.total_tokens_saved, 0),
      dedup: sessions.reduce((a, s) => a + s.dedup_hits, 0),
    };
  }, [sessions]);

  // Pressure-sorted lineage groups (subagents nested under their parent).
  const groups = useMemo(() => buildLineageGroups(sessions), [sessions]);

  const toggle = (id: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const openInspector = (s: SessionDetail) =>
    openEntity({
      kind: "session",
      corpusId: s.corpus_id,
      sessionId: s.session_id,
      seed: byId.get(s.session_id) ?? s,
    });

  const cardProps = (s: SessionDetail) => ({
    session: s,
    corpus: corpusById.get(s.corpus_id),
    samples: samples.get(s.session_id) ?? [],
    fresh: freshIds.has(s.session_id),
    expanded: expanded.has(s.session_id),
    onToggle: () => toggle(s.session_id),
    onOpenInspector: () => openInspector(s),
  });

  return (
    <AdaptiveSurface>
      <div className="h-full flex flex-col min-h-0">
        <header className="flex items-center justify-between gap-4 p-5 pb-3 shrink-0">
          <div className="min-w-0">
            <H1>Sessions</H1>
            <p className="font-sans text-sm text-text-dim mt-1">
              {agg.count === 0
                ? "No agents connected."
                : `${agg.count} live agent ${agg.count === 1 ? "session" : "sessions"}.`}
            </p>
          </div>
          {agg.count > 0 && (
            <div className="flex items-center gap-5 shrink-0">
              <AggStat label="budget">
                <span className={toneTextClass(utilizationTone(agg.util))}>
                  {clampPct(agg.util * 100)}%
                </span>
              </AggStat>
              <AggStat label="saved">
                <NumberTicker value={agg.saved} format={formatTokens} />
              </AggStat>
              <AggStat label="dedup">
                <NumberTicker value={agg.dedup} flashOnChange />
              </AggStat>
            </div>
          )}
        </header>

        <div className="flex-1 min-h-0 overflow-y-auto px-5 pb-5">
          {!loaded ? (
            <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3 items-start">
              {Array.from({ length: 6 }).map((_, i) => (
                <SessionCardSkeleton key={i} />
              ))}
            </div>
          ) : sessions.length === 0 ? (
            <div className="grid place-items-center h-full">
              <EmptyState
                icon={Users}
                accent
                title="No active sessions"
                hint={
                  <span className="block">
                    Point Claude Code, Cursor, or any MCP client at a project
                    and its agents appear here live.
                    <button
                      type="button"
                      onClick={() => navigator.clipboard.writeText(CONNECT_CMD)}
                      title="Click to copy"
                      className="mt-3 block w-full text-left rounded-md border border-border bg-surface-sunken px-3 py-2 font-mono text-mono-mini text-text break-all cursor-pointer hover:border-border-hover transition-colors duration-150"
                    >
                      {`$ ${CONNECT_CMD}`}
                    </button>
                  </span>
                }
              />
            </div>
          ) : (
            <motion.div
              variants={listContainer}
              initial="initial"
              animate="animate"
              className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3 items-start"
            >
              {groups.map((group) => (
                <LineageGroupCell
                  key={group.parent.session_id}
                  group={group}
                  cardProps={cardProps}
                />
              ))}
            </motion.div>
          )}
        </div>
      </div>
    </AdaptiveSurface>
  );
}

function AggStat({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col items-end">
      <span className="font-mono text-base font-semibold tabular-nums text-text">
        {children}
      </span>
      <span className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
        {label}
      </span>
    </div>
  );
}

type CardProps = {
  session: SessionDetail;
  corpus: CorpusInfo | undefined;
  samples: readonly SessionSample[];
  fresh: boolean;
  expanded: boolean;
  onToggle: () => void;
  onOpenInspector: () => void;
};

/** One lineage group: the parent card plus its subagents nested beneath. */
function LineageGroupCell({
  group,
  cardProps,
}: {
  group: LineageGroup;
  cardProps: (s: SessionDetail) => CardProps;
}) {
  return (
    <motion.div variants={listItem} layout className="flex flex-col gap-2">
      <SessionCard {...cardProps(group.parent)} />
      {group.children.length > 0 && (
        <div className="ml-3 pl-3 border-l border-border-soft flex flex-col gap-2">
          <span className="pl-0.5 font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
            {group.children.length} subagent
            {group.children.length === 1 ? "" : "s"}
          </span>
          {group.children.map((c) => (
            <SessionCard key={c.session_id} {...cardProps(c)} child />
          ))}
        </div>
      )}
    </motion.div>
  );
}

export function SessionCard({
  session: s,
  corpus,
  samples,
  fresh,
  expanded,
  onToggle,
  onOpenInspector,
  child = false,
}: CardProps & { child?: boolean }) {
  const tone = utilizationTone(s.utilization);
  const pct = clampPct(s.utilization * 100);
  const verdict = pressureVerdict(s.pressure_level);
  const proj = projectCritical(s, samples);
  const series = samples.map((x) => x.tokensUsed);
  const ringSize = child ? 44 : 56;

  return (
    <motion.div
      layout
      className={cn(
        "group rounded-lg border bg-surface cursor-pointer",
        "shadow-xs hover:shadow-md transition-[border-color,box-shadow] duration-200",
        tone === "danger"
          ? "border-danger/50"
          : "border-border hover:border-border-hover",
        expanded && "shadow-md",
      )}
      whileHover={expanded ? undefined : { y: -2 }}
    >
      <button
        onClick={onToggle}
        aria-expanded={expanded}
        className="w-full text-left p-4 cursor-pointer rounded-lg"
      >
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
              <span className="flex-1" />
              <ChevronDown
                className={cn(
                  "h-4 w-4 text-text-dim shrink-0 transition-transform duration-200",
                  expanded && "rotate-180",
                )}
                strokeWidth={2}
              />
            </div>
            <p
              className={cn(
                "font-mono text-mono-mini uppercase tracking-[0.06em] mt-1",
                toneTextClass(tone),
              )}
            >
              {verdict.word}
            </p>
            <p className="font-mono text-mono-mini text-text-dim mt-0.5 truncate">
              {corpus ? corpusLabel(corpus) : s.corpus_id}
              {s.client_name ? ` · ${s.client_name}` : ""}
            </p>
          </div>
        </div>

        <div className="mt-3 -mx-1">
          <Sparkline
            data={series}
            smooth
            tone={tone}
            height={32}
            ariaLabel={`Token usage trend for ${s.session_id}`}
          />
        </div>

        <div className="mt-2 flex items-center justify-between font-mono text-mono-mini text-text-dim">
          <span className={cn(fresh && "ministr-pulse rounded px-1 text-text")}>
            turn {s.current_turn}
          </span>
          <span className="tabular-nums">
            {formatTokens(s.tokens_used)} / {formatTokens(s.tokens_remaining)}{" "}
            free
          </span>
        </div>

        {proj && proj.turns != null && (
          <p
            className={cn(
              "mt-1.5 font-mono text-mono-micro uppercase tracking-[0.06em]",
              toneTextClass(tone),
            )}
          >
            ≈ {proj.turns} turn{proj.turns === 1 ? "" : "s"} to limit
          </p>
        )}
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

/** The in-place per-session dashboard revealed when a card is expanded —
 *  economics grid + burn/projection + a larger trend, no panel switch. */
function SessionExpanded({
  session,
  samples,
  onOpenInspector,
}: {
  session: SessionDetail;
  samples: readonly SessionSample[];
  onOpenInspector: () => void;
}) {
  const v = deriveVitals(session);
  if (!v) return null;
  const burn = burnRate(samples);
  const proj = projectCritical(session, samples);
  const series = samples.map((s) => s.tokensUsed);

  return (
    <div className="border-t border-border-soft px-4 py-3 flex flex-col gap-3">
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

      <Sparkline
        data={series}
        smooth
        tone={v.tone}
        height={44}
        ariaLabel={`Token usage trend for ${session.session_id}`}
      />

      <button
        onClick={onOpenInspector}
        className="self-start inline-flex items-center gap-1 font-mono text-mono-mini uppercase tracking-[0.08em] text-text-muted hover:text-text cursor-pointer transition-colors duration-150"
      >
        Open full inspector
        <ExternalLink className="h-3 w-3" strokeWidth={2} />
      </button>
    </div>
  );
}

/**
 * Loading placeholder that mirrors a SessionCard's layout (ring + title +
 * sparkline + footer) so the grid previews its structure while the first
 * poll lands — no blank gap, no layout jump (2026 skeleton-screen norm).
 */
export function SessionCardSkeleton() {
  return (
    <div
      className="rounded-lg border border-border bg-surface p-4 shadow-xs"
      aria-hidden
    >
      <div className="flex items-start gap-3">
        {/* Circle to mirror the BudgetRing; inline radius beats the
            .ministr-skeleton base radius. */}
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
