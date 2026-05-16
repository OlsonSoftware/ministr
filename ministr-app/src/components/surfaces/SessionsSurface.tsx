/**
 * SessionsSurface — the flagship: a live, motion-rich board of every
 * agent session consuming the cache, across all projects.
 *
 * Reuses the shared `useSessions` store (one poll for the whole app),
 * the `lib/sessions` derivations, and opens the deep per-session
 * inspector via the global EntityPanel. No own fetch.
 */
import { useMemo } from "react";
import { Activity, Users } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";

import type { CorpusInfo, DaemonStatus, SessionDetail } from "../../lib/types";
import { corpusLabel } from "../../lib/corpus";
import { formatTokens } from "../../lib/format";
import { toneTextClass } from "../../lib/status";
import {
  clampPct,
  pressureFromUtil,
  pressureVerdict,
  projectCritical,
  utilizationTone,
} from "../../lib/sessions";
import { listContainer, listItem, spring } from "../../lib/motion";
import { cn } from "../../lib/utils";
import { useEntityPanel } from "../../hooks/useEntityPanel";
import { useSessions } from "../../hooks/useSessions";

import { BudgetRing } from "../ui/budget-ring";
import { EmptyState } from "../ui/empty-state";
import { NumberTicker } from "../ui/number-ticker";
import { Sparkline } from "../ui/sparkline";
import { StatusDot } from "../ui/status-dot";
import { H1 } from "../ui/heading";

const CONNECT_CMD = "npx @modelcontextprotocol/inspector ministr stdio";

export function SessionsSurface({
  status,
}: {
  status: DaemonStatus;
  activeCorpusId: string | null;
}) {
  const { sessions, byId, samples, freshIds, loaded } = useSessions();
  const { openEntity } = useEntityPanel();

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
      used,
      cap,
      util: cap > 0 ? used / cap : 0,
      saved: sessions.reduce((a, s) => a + s.total_tokens_saved, 0),
      dedup: sessions.reduce((a, s) => a + s.dedup_hits, 0),
    };
  }, [sessions]);

  const open = (s: SessionDetail) =>
    openEntity({
      kind: "session",
      corpusId: s.corpus_id,
      sessionId: s.session_id,
      seed: byId.get(s.session_id) ?? s,
    });

  return (
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
          <p className="font-sans text-sm text-text-dim py-6">
            Connecting<span className="ministr-blink">_</span>
          </p>
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
            className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3"
          >
            <AnimatePresence mode="popLayout">
              {sessions.map((s) => (
                <SessionCard
                  key={s.session_id}
                  session={s}
                  corpus={corpusById.get(s.corpus_id)}
                  series={(samples.get(s.session_id) ?? []).map(
                    (x) => x.tokensUsed,
                  )}
                  fresh={freshIds.has(s.session_id)}
                  onOpen={() => open(s)}
                />
              ))}
            </AnimatePresence>
          </motion.div>
        )}
      </div>
    </div>
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

function SessionCard({
  session: s,
  corpus,
  series,
  fresh,
  onOpen,
}: {
  session: SessionDetail;
  corpus: CorpusInfo | undefined;
  series: number[];
  fresh: boolean;
  onOpen: () => void;
}) {
  const tone = utilizationTone(s.utilization);
  const pct = clampPct(s.utilization * 100);
  const verdict = pressureVerdict(s.pressure_level);
  const proj = projectCritical(s, []);

  return (
    <motion.button
      layout
      layoutId={`session-${s.session_id}`}
      variants={listItem}
      exit="exit"
      transition={spring}
      onClick={onOpen}
      whileHover={{ y: -2 }}
      className={cn(
        "group text-left rounded-lg border bg-surface p-4 cursor-pointer",
        "transition-colors duration-200",
        tone === "danger"
          ? "border-danger/50"
          : "border-border hover:border-border-hover",
      )}
    >
      <div className="flex items-start gap-3">
        <BudgetRing
          utilization={s.utilization}
          pressure={pressureFromUtil(s.utilization)}
          size={56}
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
          {formatTokens(s.tokens_used)} / {formatTokens(s.tokens_remaining)} free
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
    </motion.button>
  );
}
