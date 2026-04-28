import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Users,
  TrendingDown,
  Copy,
  Search,
  Filter,
  Radio,
  Gauge,
} from "lucide-react";
import { Card } from "./ui/card";
import { Button } from "./ui/button";
import { Badge } from "./ui/badge";
import { BudgetRing } from "./ui/budget-ring";
import { StatusDot } from "./ui/status-dot";
import { TurnBlock } from "./ui/turn-block";
import { VitalCard } from "./ui/vital-card";
import { cn } from "../lib/utils";
import { accentTone, labelSmallCap } from "../lib/ui-tokens";
import { formatTokens } from "../lib/format";
import { corpusLabelById } from "../lib/corpus";
import type { SessionDetail, DaemonStatus } from "../lib/types";

type PressureFilter = "all" | "elevated" | "critical";

const PRESSURE_ORDER = ["none", "low", "medium", "high", "critical"] as const;
type Pressure = (typeof PRESSURE_ORDER)[number];

interface Props {
  status: DaemonStatus;
}

export function SessionDashboard({ status }: Props) {
  const [sessions, setSessions] = useState<SessionDetail[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [query, setQuery] = useState("");
  const [pressureFilter, setPressureFilter] = useState<PressureFilter>("all");
  const [freshSessions, setFreshSessions] = useState<Set<string>>(new Set());
  const prevTurns = useRef<Map<string, number>>(new Map());

  useEffect(() => {
    let cancelled = false;
    async function load() {
      try {
        const s = await invoke<SessionDetail[]>("list_sessions");
        if (cancelled) return;

        const fresh = new Set<string>();
        for (const sess of s) {
          const prev = prevTurns.current.get(sess.session_id);
          if (prev !== undefined && sess.current_turn > prev) {
            fresh.add(sess.session_id);
          }
          prevTurns.current.set(sess.session_id, sess.current_turn);
        }
        setSessions(s);
        setLoaded(true);
        if (fresh.size) {
          setFreshSessions(fresh);
          setTimeout(() => {
            if (!cancelled) setFreshSessions(new Set());
          }, 1500);
        }
      } catch {
        if (!cancelled) setLoaded(true);
      }
    }
    load();
    const interval = setInterval(load, 1500);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, []);

  const vitals = useMemo(() => {
    const total = sessions.length;
    const tokensUsed = sessions.reduce((s, x) => s + x.tokens_used, 0);
    const capacity = sessions.reduce(
      (s, x) => s + x.tokens_used + x.tokens_remaining,
      0,
    );
    const util = capacity > 0 ? tokensUsed / capacity : 0;
    const saved = sessions.reduce((s, x) => s + x.total_tokens_saved, 0);
    const dedup = sessions.reduce((s, x) => s + x.dedup_hits, 0);
    const pressure = sessions.reduce<Pressure>((max, x) => {
      const p = (x.pressure_level as Pressure) ?? "none";
      return PRESSURE_ORDER.indexOf(p) > PRESSURE_ORDER.indexOf(max)
        ? p
        : max;
    }, "none");
    return { total, tokensUsed, capacity, util, saved, dedup, pressure };
  }, [sessions]);

  const filtered = useMemo(() => {
    let list = sessions;
    if (pressureFilter === "elevated") {
      list = list.filter((s) =>
        ["medium", "high", "critical"].includes(s.pressure_level),
      );
    } else if (pressureFilter === "critical") {
      list = list.filter((s) => s.pressure_level === "critical");
    }
    const q = query.trim().toLowerCase();
    if (q) {
      list = list.filter((s) =>
        [
          s.session_id,
          s.corpus_id,
          corpusLabelById(status.corpora, s.corpus_id),
          s.client_name ?? "",
        ].some((f) => f.toLowerCase().includes(q)),
      );
    }
    return list;
  }, [sessions, pressureFilter, query, status.corpora]);

  // Group filtered sessions into a parent/subagent tree. A subagent
  // whose parent dropped out of `filtered` (e.g. via a query that
  // matched only the child) gets re-attached to the full sessions list
  // by id; if the parent really is missing (different corpus, etc.)
  // the subagent is rendered as an "orphan" top-level entry. This
  // keeps the hierarchy coherent under filtering instead of having
  // children appear floating without context.
  const tree = useMemo(() => {
    type Node = { session: SessionDetail; subagents: SessionDetail[] };
    const byId = new Map(sessions.map((s) => [s.session_id, s]));
    const filteredIds = new Set(filtered.map((s) => s.session_id));
    const nodes = new Map<string, Node>();
    const orphans: Node[] = [];

    for (const s of filtered) {
      if (!s.parent_session_id) {
        if (!nodes.has(s.session_id)) {
          nodes.set(s.session_id, { session: s, subagents: [] });
        } else {
          nodes.get(s.session_id)!.session = s;
        }
      }
    }
    for (const s of filtered) {
      if (s.parent_session_id) {
        const existing = nodes.get(s.parent_session_id);
        if (existing) {
          existing.subagents.push(s);
        } else if (filteredIds.has(s.parent_session_id)) {
          // Parent is in the filtered list but hasn't been added yet —
          // shouldn't happen with the loops above, but guard anyway.
          continue;
        } else {
          // Parent dropped out of the filter — re-attach if we can.
          const parent = byId.get(s.parent_session_id);
          if (parent) {
            const node = nodes.get(parent.session_id) ?? {
              session: parent,
              subagents: [],
            };
            node.subagents.push(s);
            nodes.set(parent.session_id, node);
          } else {
            orphans.push({ session: s, subagents: [] });
          }
        }
      }
    }
    return [...nodes.values(), ...orphans];
  }, [filtered, sessions]);

  return (
    <div className="space-y-4 ministr-fade-in">
      <header className="flex items-end justify-between gap-4 flex-wrap">
        <div>
          <h1 className="text-lg font-semibold text-text flex items-center gap-2">
            <Users className="h-4 w-4 text-accent" />
            Sessions
          </h1>
          <p className="text-xs text-text-dim mt-0.5">
            Every MCP agent attached to the daemon — live.
          </p>
        </div>
        <div className="flex items-center gap-2">
          {vitals.total > 0 && (
            <Badge variant="success" dot>
              {vitals.total} active
            </Badge>
          )}
          {vitals.pressure === "critical" && (
            <Badge variant="danger" dot>
              critical pressure
            </Badge>
          )}
        </div>
      </header>

      {/* Vitals row — aligns with Overview vitals */}
      {vitals.total > 0 && (
        <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
          <VitalCard
            title="Aggregate budget"
            subtitle="Tokens used across active sessions"
            layout="center"
          >
            <BudgetRing
              utilization={vitals.util}
              pressure={vitals.pressure}
              size={108}
              stroke={8}
            >
              <span className="font-mono text-xl font-bold tabular-nums text-text leading-none">
                {(vitals.util * 100).toFixed(0)}
                <span className="text-sm text-text-dim">%</span>
              </span>
              <span className="text-[10px] uppercase tracking-wider text-text-dim mt-1">
                {formatTokens(vitals.tokensUsed)} /{" "}
                {formatTokens(vitals.capacity)}
              </span>
            </BudgetRing>
          </VitalCard>

          <VitalCard
            title="Tokens saved"
            subtitle="From dedup + delta + compression"
            layout="center"
            right={
              <Badge variant="success" className="gap-1">
                <TrendingDown className="h-2.5 w-2.5" />
                {formatTokens(vitals.saved)}
              </Badge>
            }
          >
            <div className="flex items-center gap-4 h-[108px]">
              <div className="flex flex-col">
                <span className="font-mono text-3xl font-bold tabular-nums text-success leading-none">
                  {formatTokens(vitals.saved)}
                </span>
                <span className="text-[10px] uppercase tracking-wider text-text-dim mt-1.5">
                  across {vitals.total}{" "}
                  {vitals.total === 1 ? "session" : "sessions"}
                </span>
              </div>
            </div>
          </VitalCard>

          <VitalCard
            title="Dedup hits"
            subtitle="Sections ministr skipped resending"
            layout="center"
          >
            <div className="flex items-center gap-4 h-[108px]">
              <div className="flex flex-col">
                <span className="font-mono text-3xl font-bold tabular-nums text-accent leading-none">
                  {vitals.dedup.toLocaleString()}
                </span>
                <span className="text-[10px] uppercase tracking-wider text-text-dim mt-1.5">
                  cache hits on re-read
                </span>
              </div>
              <div className="grid h-14 w-14 place-items-center rounded-xl bg-[var(--color-accent-soft)] text-accent">
                <Copy className="h-6 w-6" />
              </div>
            </div>
          </VitalCard>
        </div>
      )}

      {/* Filter bar */}
      {sessions.length > 0 && (
        <div className="flex items-center gap-2 flex-wrap">
          <div className="relative flex-1 min-w-[200px] max-w-xs">
            <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-text-dim" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Filter by session or corpus…"
              className="h-8 w-full pl-8 pr-2.5 text-xs rounded-lg border border-border/70 bg-surface-raised text-text placeholder:text-text-dim font-mono focus:outline-none focus:border-[var(--color-accent-ring)]"
            />
          </div>
          <div className="flex items-center gap-0.5 rounded-lg border border-border/70 bg-surface-raised p-0.5">
            <FilterPill
              active={pressureFilter === "all"}
              onClick={() => setPressureFilter("all")}
            >
              <Filter className="h-3 w-3" />
              All
            </FilterPill>
            <FilterPill
              active={pressureFilter === "elevated"}
              onClick={() => setPressureFilter("elevated")}
            >
              Elevated+
            </FilterPill>
            <FilterPill
              active={pressureFilter === "critical"}
              onClick={() => setPressureFilter("critical")}
            >
              Critical
            </FilterPill>
          </div>
          <span className="text-[11px] text-text-dim font-mono ml-auto">
            {filtered.length} / {sessions.length}
          </span>
        </div>
      )}

      {/* Session list — uses the shared TurnBlock primitive */}
      <section>
        <div className="flex items-center justify-between mb-2">
          <h2 className={cn(labelSmallCap, "flex items-center gap-1.5")}>
            <Radio className="h-3 w-3" />
            Active sessions
            {vitals.total > 0 && (
              <span className="inline-flex items-center gap-1 ml-2 text-[10px] text-accent normal-case font-sans font-medium">
                <StatusDot tone="accent" pulse="live" />
                streaming
              </span>
            )}
          </h2>
        </div>

        {!loaded ? (
          <div className="flex items-center justify-center py-12">
            <div className="animate-spin h-7 w-7 rounded-full border-2 border-border border-t-accent" />
          </div>
        ) : sessions.length === 0 ? (
          <EmptyState />
        ) : filtered.length === 0 ? (
          <Card className="flex flex-col items-center gap-2 py-10 text-center">
            <div className="grid h-10 w-10 place-items-center rounded-lg bg-surface-overlay text-text-dim">
              <Filter className="h-4 w-4" />
            </div>
            <p className="text-sm font-medium text-text">No matching sessions</p>
            <p className="text-xs text-text-dim max-w-xs">
              Clear the filter or widen the pressure scope to see more.
            </p>
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                setQuery("");
                setPressureFilter("all");
              }}
              className="mt-1"
            >
              Reset filters
            </Button>
          </Card>
        ) : (
          <div className="space-y-2">
            {tree.map((node) => (
              <div key={node.session.session_id} className="space-y-1.5">
                <TurnBlock
                  session={node.session}
                  corpora={status.corpora}
                  fresh={freshSessions.has(node.session.session_id)}
                />
                {node.subagents.length > 0 && (
                  <div className="ml-4 border-l border-border/50 pl-3 space-y-1.5">
                    {node.subagents.map((sub) => (
                      <TurnBlock
                        key={sub.session_id}
                        session={sub}
                        corpora={status.corpora}
                        fresh={freshSessions.has(sub.session_id)}
                      />
                    ))}
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}

function FilterPill({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "inline-flex items-center gap-1 px-2.5 py-1 text-[11px] font-medium rounded-md transition-all duration-120 cursor-pointer",
        active
          ? accentTone
          : "text-text-muted hover:text-text hover:bg-surface-overlay/60",
      )}
    >
      {children}
    </button>
  );
}

function EmptyState() {
  return (
    <Card className="flex flex-col items-center gap-3 py-12 text-center">
      <div className="grid h-12 w-12 place-items-center rounded-xl bg-surface-overlay text-text-dim">
        <Users className="h-5 w-5" />
      </div>
      <div className="space-y-1">
        <p className="text-sm font-medium text-text">No active sessions</p>
        <p className="max-w-sm text-xs text-text-dim">
          Point Claude Code, Cursor, or any MCP client at{" "}
          <span className="font-mono">~/.ministr/ministrd.sock</span> and sessions
          stream in here with live budget, dedup, and compression metrics.
        </p>
      </div>
    </Card>
  );
}

// Hide unused-import warnings for icons used above.
export const __icons = { Gauge };
