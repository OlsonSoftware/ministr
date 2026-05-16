/**
 * ProjectSessions — live MCP sessions for a single project, rendered in
 * the Projects detail pane.
 *
 * Reads the shared `useSessions` store (one poll for the whole app) and
 * derives this project's slice with a memo — no own fetch, no remount on
 * project switch. The vitals are a distinct rollup *band* (aggregate ring
 * + worst-session callout) so the summary no longer looks like just
 * another card.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, Users } from "lucide-react";

import type { CorpusInfo } from "../../lib/types";
import { formatTokens } from "../../lib/format";
import { toneTextClass, type Tone } from "../../lib/status";
import {
  clampPct,
  pressureFromUtil,
  pressureVerdict,
  utilizationTone,
} from "../../lib/sessions";
import { cn } from "../../lib/utils";
import { useEntityPanel } from "../../hooks/useEntityPanel";
import { useSessions } from "../../hooks/useSessions";

import { BudgetRing } from "../ui/budget-ring";
import { EmptyState } from "../ui/empty-state";
import { StatusDot } from "../ui/status-dot";
import { TurnBlock } from "../ui/turn-block";

const CONNECT_CMD = "npx @modelcontextprotocol/inspector ministr stdio";

export function ProjectSessions({ corpus }: { corpus: CorpusInfo }) {
  const { openEntity } = useEntityPanel();
  const { sessions: all, byId, freshIds, loaded, lastSyncMs } = useSessions();

  const sessions = useMemo(
    () => all.filter((s) => s.corpus_id === corpus.id),
    [all, corpus.id],
  );

  // Heartbeat — a single hard flash each time a poll lands.
  const [beat, setBeat] = useState(false);
  const prevSync = useRef(0);
  useEffect(() => {
    if (lastSyncMs && lastSyncMs !== prevSync.current) {
      prevSync.current = lastSyncMs;
      setBeat(true);
      const t = setTimeout(() => setBeat(false), 250);
      return () => clearTimeout(t);
    }
  }, [lastSyncMs]);

  const vitals = useMemo(() => {
    const total = sessions.length;
    const tokensUsed = sessions.reduce((a, s) => a + s.tokens_used, 0);
    const capacity = sessions.reduce(
      (a, s) => a + s.tokens_used + s.tokens_remaining,
      0,
    );
    const util = capacity > 0 ? tokensUsed / capacity : 0;
    const saved = sessions.reduce((a, s) => a + s.total_tokens_saved, 0);
    const dedup = sessions.reduce((a, s) => a + s.dedup_hits, 0);
    // Worst = highest individual utilization (the one to watch).
    const worst = sessions.reduce<(typeof sessions)[number] | null>(
      (w, s) => (w === null || s.utilization > w.utilization ? s : w),
      null,
    );
    return { total, tokensUsed, capacity, util, saved, dedup, worst };
  }, [sessions]);

  const worst = vitals.worst;
  const worstTone = worst ? utilizationTone(worst.utilization) : "muted";
  const worstSevere = worstTone === "warning" || worstTone === "danger";

  return (
    <section className="pt-3 border-t border-border-soft">
      {/* Section header — label · live pill · poll heartbeat */}
      <div className="flex items-center gap-2 mb-3">
        <h3 className="font-mono text-mono-mini font-semibold uppercase tracking-[0.05em] text-text">
          Sessions
        </h3>
        {vitals.total > 0 && (
          <span className="inline-flex items-center gap-1.5 rounded-full border border-success/40 bg-success/10 px-2 py-0.5 font-mono text-mono-mini font-medium uppercase tracking-[0.06em] text-success">
            <StatusDot tone="success" pulse="live" />
            {vitals.total} live
          </span>
        )}
        <div className="flex-1" />
        <span className="font-mono text-mono-mini uppercase tracking-[0.05em] text-text-dim">
          poll
        </span>
        <span
          className={cn(
            "h-1.5 w-1.5 rounded-full transition-colors duration-200",
            beat ? "bg-accent" : "bg-border",
          )}
          aria-label="Polling heartbeat"
        />
      </div>

      {!loaded ? (
        <p className="font-serif text-base italic text-text-dim py-4">
          Loading<span className="ministr-blink">_</span>
        </p>
      ) : sessions.length === 0 ? (
        <EmptyState
          icon={Users}
          title="No active sessions"
          hint={
            <span className="block">
              Point Claude Code, Cursor, or any MCP client at this project and
              its agents appear here live — budget, pressure, and dedup.
              <button
                type="button"
                onClick={() => navigator.clipboard.writeText(CONNECT_CMD)}
                title="Click to copy"
                className="mt-2 block w-full text-left border border-border-soft bg-surface-sunken px-2.5 py-1.5 font-mono text-mono-mini not-italic text-text break-all cursor-pointer hover:border-border-hover hover:bg-surface-overlay transition-colors duration-150"
              >
                {`> ${CONNECT_CMD}`}
              </button>
            </span>
          }
        />
      ) : (
        <>
          {/* Rollup band — aggregate ring + vitals + worst-session callout.
              Taller and ring-led so it reads as a summary, not a card. */}
          <div className="border border-border-soft bg-surface flex items-stretch mb-3">
            <div className="flex items-center justify-center px-3 border-r border-border-soft shrink-0">
              <BudgetRing
                utilization={vitals.util}
                pressure={pressureFromUtil(vitals.util)}
                size={52}
                stroke={6}
              >
                <span className="font-mono text-sm font-bold tabular-nums text-text leading-none">
                  {clampPct(vitals.util * 100)}
                  <span className="text-mono-mini text-text-dim">%</span>
                </span>
              </BudgetRing>
            </div>
            <div className="flex flex-col flex-1 min-w-0">
              <div className="flex items-stretch h-9 border-b border-border-soft">
                <VStat
                  label="Budget"
                  value={`${clampPct(vitals.util * 100)}%`}
                />
                <VStat
                  label="Tokens"
                  value={`${formatTokens(vitals.tokensUsed)} / ${formatTokens(vitals.capacity)}`}
                />
                <VStat
                  label="Saved"
                  value={formatTokens(vitals.saved)}
                  tone="success"
                />
                <VStat
                  label="Dedup"
                  value={vitals.dedup.toLocaleString()}
                  tone="accent"
                />
              </div>
              <button
                type="button"
                disabled={!worst}
                onClick={() =>
                  worst &&
                  openEntity({
                    kind: "session",
                    corpusId: worst.corpus_id,
                    sessionId: worst.session_id,
                    seed: worst,
                  })
                }
                className={cn(
                  "flex items-center gap-2 px-3 py-1.5 text-left min-w-0 transition-none",
                  worst && "cursor-pointer hover:bg-surface-overlay",
                )}
              >
                {worstSevere ? (
                  <AlertTriangle
                    className={cn("h-3.5 w-3.5 shrink-0", toneTextClass(worstTone))}
                    strokeWidth={2.5}
                  />
                ) : (
                  <StatusDot tone={worstTone} />
                )}
                <span className="font-mono text-mono-mini uppercase tracking-[0.05em] text-text-dim shrink-0">
                  Watch
                </span>
                {worst ? (
                  <span className="font-mono text-mono-mini tabular-nums truncate text-text">
                    {worst.session_id.slice(0, 8)}
                    <span className="text-text-dim">
                      {" "}
                      · {pressureVerdict(worst.pressure_level).word} ·{" "}
                      {clampPct(worst.utilization * 100)}%
                    </span>
                  </span>
                ) : (
                  <span className="font-mono text-mono-mini text-text-dim">
                    —
                  </span>
                )}
                <div className="flex-1" />
                {worst && (
                  <span className="font-mono text-mono-mini uppercase tracking-[0.05em] text-accent shrink-0">
                    open ›
                  </span>
                )}
              </button>
            </div>
          </div>

          {/* Card grid — one column, two on a wide detail pane. */}
          <div className="@container/psess">
            <div className="grid grid-cols-1 @[40rem]/psess:grid-cols-2 gap-2">
              {sessions.map((s) => (
                <TurnBlock
                  key={s.session_id}
                  session={s}
                  corpora={[corpus]}
                  fresh={freshIds.has(s.session_id)}
                  onClick={() =>
                    openEntity({
                      kind: "session",
                      corpusId: s.corpus_id,
                      sessionId: s.session_id,
                      seed: byId.get(s.session_id) ?? s,
                    })
                  }
                />
              ))}
            </div>
          </div>
        </>
      )}
    </section>
  );
}

function VStat({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: Tone;
}) {
  return (
    <div className="flex items-center gap-2 px-3 py-1 border-r border-border-soft last:border-r-0 min-w-0 flex-1">
      <span className="font-mono text-mono-mini uppercase tracking-[0.05em] text-text-dim shrink-0">
        {label}
      </span>
      <span
        className={cn(
          "font-mono text-sm font-semibold tabular-nums truncate",
          tone ? toneTextClass(tone) : "text-text",
        )}
      >
        {value}
      </span>
    </div>
  );
}
