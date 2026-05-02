import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useEntityPanel, type Entity } from "../../hooks/useEntityPanel";
import { BudgetRing } from "../ui/budget-ring";
import { EntityRow } from "./EntityRow";
import {
  EntitySection,
  EntitySectionEmpty,
  EntitySectionLoading,
} from "./EntitySection";
import { formatTokens } from "../../lib/format";
import { pressureTone, toneTextClass } from "../../lib/status";
import { cn } from "../../lib/utils";
import { relative } from "../../lib/time";
import type {
  ActivityEvent,
  CorpusInfo,
  SessionDetail,
} from "../../lib/types";

interface Props {
  entity: Extract<Entity, { kind: "session" }>;
}

export function SessionView({ entity }: Props) {
  const { corpusId, sessionId } = entity;
  const { openEntity } = useEntityPanel();

  const [session, setSession] = useState<SessionDetail | null>(null);
  const [activity, setActivity] = useState<ActivityEvent[] | null>(null);
  const [corpus, setCorpus] = useState<CorpusInfo | null>(null);

  useEffect(() => {
    let cancelled = false;
    setSession(null);
    setActivity(null);
    setCorpus(null);

    Promise.allSettled([
      invoke<SessionDetail[]>("list_sessions"),
      // The daemon doesn't filter activity server-side yet, so we pull
      // a generous candidate window and narrow client-side. 500 keeps
      // older session history visible on busy daemons without bloating
      // the JSON payload (display still slices to 50 below).
      invoke<ActivityEvent[]>("recent_activity", { limit: 500 }),
      invoke<CorpusInfo[]>("list_corpora"),
    ]).then(([s, a, c]) => {
      if (cancelled) return;
      setSession(
        s.status === "fulfilled"
          ? s.value.find((x) => x.session_id === sessionId) ?? null
          : null,
      );
      setActivity(
        a.status === "fulfilled"
          ? a.value.filter((e) => e.session_id === sessionId)
          : [],
      );
      setCorpus(
        c.status === "fulfilled"
          ? c.value.find((x) => x.id === corpusId) ?? null
          : null,
      );
    });
    return () => {
      cancelled = true;
    };
  }, [sessionId, corpusId]);

  const tone = session ? pressureTone(session.pressure_level) : "muted";
  const utilPct = session ? (session.utilization * 100).toFixed(0) : "—";

  return (
    <div className="flex flex-col gap-4">
      {/* §1 Overview */}
      <EntitySection chapter={1} title="Overview">
        <div className="px-3 py-3 space-y-1.5">
          <div className="flex items-baseline gap-2 flex-wrap">
            <span className="font-mono text-base font-bold text-text break-all">
              {sessionId}
            </span>
            {session && (
              <span
                className={cn(
                  "font-mono text-[0.6875rem] font-semibold uppercase tracking-[0.05em]",
                  toneTextClass(tone),
                )}
              >
                {session.pressure_level}
              </span>
            )}
          </div>
          {session?.client_name && (
            <p className="font-sans text-xs text-text-dim">
              <span className="font-mono uppercase tracking-[0.05em] text-[0.6875rem]">Client</span>{" "}
              · {session.client_name}
            </p>
          )}
        </div>
        {session && (
          <div className="grid grid-cols-4 border-t border-border-soft">
            <Stat label="Turn" value={session.current_turn.toString()} />
            <Stat label="Budget" value={`${utilPct}%`} />
            <Stat
              label="Tokens"
              value={formatTokens(session.tokens_used)}
            />
            <Stat
              label="Saved"
              value={formatTokens(session.total_tokens_saved)}
              tone="success"
            />
          </div>
        )}
      </EntitySection>

      {/* §2 Budget */}
      {session && (
        <EntitySection chapter={2} title="Budget">
          <div className="flex items-center gap-4 px-3 py-3">
            <BudgetRing
              utilization={session.utilization}
              pressure={
                session.pressure_level as
                  | "none"
                  | "low"
                  | "medium"
                  | "high"
                  | "critical"
              }
              size={96}
              stroke={8}
            >
              <span className="font-mono text-xl font-bold tabular-nums text-text leading-none">
                {utilPct}
                <span className="text-sm text-text-dim">%</span>
              </span>
            </BudgetRing>
            <div className="space-y-1 font-sans text-sm text-text-muted">
              <p><span className="font-mono text-[0.6875rem] uppercase tracking-[0.05em] text-text-dim">Used</span> · {formatTokens(session.tokens_used)}</p>
              <p><span className="font-mono text-[0.6875rem] uppercase tracking-[0.05em] text-text-dim">Free</span> · {formatTokens(session.tokens_remaining)}</p>
              <p className="text-success"><span className="font-mono text-[0.6875rem] uppercase tracking-[0.05em] opacity-70">Saved</span> · {formatTokens(session.total_tokens_saved)}</p>
              <p className="text-accent"><span className="font-mono text-[0.6875rem] uppercase tracking-[0.05em] opacity-70">Dedup hits</span> · {session.dedup_hits}</p>
            </div>
          </div>
        </EntitySection>
      )}

      {/* §3 Activity */}
      <EntitySection
        chapter={3}
        title="Activity"
        meta={activity === null ? "…" : activity.length}
      >
        {activity === null ? (
          <EntitySectionLoading />
        ) : activity.length === 0 ? (
          <EntitySectionEmpty label="No recorded activity for this session." />
        ) : (
          <div className="max-h-80 overflow-y-auto">
            {activity.slice(0, 50).map((e, i) => (
              <EntityRow
                key={i}
                tag={e.tool.replace(/^ministr_/, "").toUpperCase()}
                name={e.summary || e.corpus_id}
                subtitle={
                  typeof e.tokens_delta === "number"
                    ? `+${formatTokens(e.tokens_delta)}${e.cache_hit ? " · cache hit" : ""}`
                    : e.cache_hit
                      ? "cache hit"
                      : undefined
                }
                meta={relative(Date.now(), e.timestamp_ms)}
              />
            ))}
          </div>
        )}
      </EntitySection>

      {/* §4 Corpus */}
      {corpus && (
        <EntitySection chapter={4} title="Corpus">
          <EntityRow
            tag="corpus"
            name={corpus.id}
            subtitle={corpus.paths[0]}
            meta={`${corpus.sections_count.toLocaleString()}§`}
            onClick={() => openEntity({ kind: "corpus", corpus })}
          />
        </EntitySection>
      )}
    </div>
  );
}

function Stat({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: "success" | "accent";
}) {
  return (
    <div className="border-r border-border-soft last:border-r-0 px-3 py-2">
      <p className="font-mono text-[0.6875rem] uppercase tracking-[0.05em] text-text-dim">
        {label}
      </p>
      <p
        className={cn(
          "font-mono text-base font-semibold tabular-nums mt-0.5",
          tone === "success" && "text-success",
          tone === "accent" && "text-accent",
          !tone && "text-text",
        )}
      >
        {value}
      </p>
    </div>
  );
}
