import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, ChevronLeft, ChevronRight, Filter, Users } from "lucide-react";
import { Button } from "./ui/button";
import { useEntityPanel } from "../hooks/useEntityPanel";
import { EmptyState } from "./ui/empty-state";
import { StatusDot } from "./ui/status-dot";
import { TurnBlock } from "./ui/turn-block";
import { ActivityFeed } from "./ui/activity-feed";
import { CoherenceFeed } from "./ui/coherence-feed";
import { cn } from "../lib/utils";
import { formatTokens } from "../lib/format";
import { corpusLabelById } from "../lib/corpus";
import {
  pressureTone,
  toneTextClass,
  type Tone,
} from "../lib/status";
import type {
  ActivityEvent,
  CoherenceEvent,
  CorpusInfo,
  DaemonStatus,
  SessionDetail,
} from "../lib/types";

type PressureFilter = "all" | "elevated" | "critical";
type View = "live" | "history";

const PRESSURE_ORDER = ["none", "low", "medium", "high", "critical"] as const;
type Pressure = (typeof PRESSURE_ORDER)[number];

const HISTORY_WINDOW_MS = 24 * 3600 * 1000;
const DRAWER_KEY = "ministr-sessions-drawer-open";
/** Versioned localStorage key for ended-session history. Bump the suffix
 *  when SessionDetail's wire shape changes so we discard incompatible
 *  cached entries instead of crashing the dashboard. */
const HISTORY_KEY = "ministr-sessions-history-v1";

interface Props {
  status: DaemonStatus;
}

interface HistoryEntry {
  endedAt: number;
  session: SessionDetail;
}

/** Load persisted ended-session history, dropping entries older than the
 *  24h window. Returns [] on any read/parse failure — bad cached state
 *  must never block the dashboard from rendering. */
function loadPersistedHistory(): HistoryEntry[] {
  try {
    const raw = localStorage.getItem(HISTORY_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const now = Date.now();
    return parsed.filter(
      (e): e is HistoryEntry =>
        e &&
        typeof e === "object" &&
        typeof e.endedAt === "number" &&
        e.session &&
        typeof e.session.session_id === "string" &&
        now - e.endedAt < HISTORY_WINDOW_MS,
    );
  } catch {
    return [];
  }
}

export function SessionDashboard({ status }: Props) {
  const { openEntity } = useEntityPanel();
  const [sessions, setSessions] = useState<SessionDetail[]>([]);
  const [activity, setActivity] = useState<ActivityEvent[]>([]);
  const [coherence, setCoherence] = useState<CoherenceEvent[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [query, setQuery] = useState("");
  const [pressureFilter, setPressureFilter] = useState<PressureFilter>("all");
  const [view, setView] = useState<View>("live");
  const [compact, setCompact] = useState(false);
  const [drawerOpen, setDrawerOpen] = useState<boolean>(() => {
    try {
      return localStorage.getItem(DRAWER_KEY) === "1";
    } catch {
      return false;
    }
  });
  const [history, setHistory] = useState<HistoryEntry[]>(loadPersistedHistory);
  const [freshSessions, setFreshSessions] = useState<Set<string>>(new Set());
  const [flashSince, setFlashSince] = useState<number | undefined>(undefined);
  const [heartbeat, setHeartbeat] = useState(false);
  const prevTurns = useRef<Map<string, number>>(new Map());
  const prevSessions = useRef<Map<string, SessionDetail>>(new Map());
  const prevSnap = useRef<number>(Date.now());
  const focusRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    try {
      localStorage.setItem(DRAWER_KEY, drawerOpen ? "1" : "0");
    } catch {
      /* ignore */
    }
  }, [drawerOpen]);

  // Persist ended-session history across reloads. We write on every change;
  // the array is bounded by the 24h window so it stays small. Wrap in a
  // try because Tauri's webview may have storage quotas in some configs.
  useEffect(() => {
    try {
      localStorage.setItem(HISTORY_KEY, JSON.stringify(history));
    } catch {
      /* ignore */
    }
  }, [history]);

  useEffect(() => {
    let cancelled = false;
    async function load() {
      try {
        const [s, a, c] = await Promise.all([
          invoke<SessionDetail[]>("list_sessions"),
          invoke<ActivityEvent[]>("recent_activity", { limit: 30 }).catch(
            () => [] as ActivityEvent[],
          ),
          invoke<CoherenceEvent[]>("recent_coherence_events", {
            limit: 20,
          }).catch(() => [] as CoherenceEvent[]),
        ]);
        if (cancelled) return;

        // Detect ended sessions and push them into history.
        const newIds = new Set(s.map((x) => x.session_id));
        const now = Date.now();
        const ended: HistoryEntry[] = [];
        for (const [id, prev] of prevSessions.current.entries()) {
          if (!newIds.has(id)) {
            ended.push({ endedAt: now, session: prev });
          }
        }
        if (ended.length > 0) {
          setHistory((h) =>
            [...ended, ...h].filter(
              (e) => now - e.endedAt < HISTORY_WINDOW_MS,
            ),
          );
        }
        prevSessions.current = new Map(s.map((x) => [x.session_id, x]));

        const fresh = new Set<string>();
        for (const sess of s) {
          const prev = prevTurns.current.get(sess.session_id);
          if (prev !== undefined && sess.current_turn > prev) {
            fresh.add(sess.session_id);
          }
          prevTurns.current.set(sess.session_id, sess.current_turn);
        }

        setSessions(s);
        setActivity(a);
        setCoherence(c);
        setFlashSince(prevSnap.current);
        prevSnap.current = now;
        setLoaded(true);

        // Heartbeat — hard-blink on each tick.
        setHeartbeat(true);
        setTimeout(() => {
          if (!cancelled) setHeartbeat(false);
        }, 250);

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
      return PRESSURE_ORDER.indexOf(p) > PRESSURE_ORDER.indexOf(max) ? p : max;
    }, "none");
    return { total, tokensUsed, capacity, util, saved, dedup, pressure };
  }, [sessions]);

  const criticalSession = sessions.find((s) => s.pressure_level === "critical");

  const sourceList: SessionDetail[] = useMemo(() => {
    if (view === "live") return sessions;
    return history.map((h) => h.session);
  }, [view, sessions, history]);

  const filtered = useMemo(() => {
    let list = sourceList;
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
  }, [sourceList, pressureFilter, query, status.corpora]);

  function inspectCritical() {
    if (!criticalSession) return;
    setPressureFilter("critical");
    setTimeout(() => {
      focusRef.current?.scrollIntoView({ block: "start", behavior: "auto" });
    }, 30);
  }

  return (
    <div className="@container/page flex h-full gap-3 min-h-0">
      {/* MAIN */}
      <div className="flex-1 min-w-0 min-h-0 flex flex-col gap-3">
        {/* Critical banner */}
        {criticalSession && (
          <button
            onClick={inspectCritical}
            className="flex items-center gap-3 border-[3px] border-danger bg-surface px-4 py-3 cursor-pointer transition-none hover:bg-danger hover:text-[var(--color-accent-fg-on)] shrink-0"
          >
            <span className="grid h-6 w-6 place-items-center bg-danger text-[var(--color-accent-fg-on)] shrink-0">
              <AlertTriangle className="h-3.5 w-3.5" strokeWidth={2.5} />
            </span>
            <span className="flex flex-col items-start min-w-0 flex-1 text-left">
              <span className="font-sans text-[0.6875rem] font-bold tracking-[0.05em] text-danger">
                Critical · 1 session over budget
              </span>
              <span className="font-mono text-xs uppercase tracking-[0.05em] text-text-dim truncate">
                {criticalSession.session_id.slice(0, 8)} ·{" "}
                {corpusLabelById(status.corpora, criticalSession.corpus_id)}
              </span>
            </span>
            <span className="font-sans text-xs font-bold tracking-[0.05em] text-danger shrink-0">
              Click to inspect →
            </span>
          </button>
        )}

        {/* Header */}
        <header className="flex items-center justify-between gap-4 shrink-0">
          <div>
            <h1 className="font-serif text-2xl font-normal text-text leading-tight ">
              Sessions
            </h1>
            <p className="font-serif text-sm italic text-text-dim mt-1">
              Live MCP agents · cache observability
            </p>
          </div>
          {vitals.total > 0 && (
            <span
              className="inline-flex items-center gap-2 border border-success bg-surface px-2 py-0.5 font-mono text-[0.6875rem] font-semibold uppercase tracking-[0.05em] text-success"
              style={{ borderRadius: "var(--radius-pill)" }}
            >
              <StatusDot tone="success" pulse="live" />
              {vitals.total} live
            </span>
          )}
        </header>

        {/* Slim vitals strip */}
        {vitals.total > 0 && (
          <div className="border border-border-soft bg-surface flex items-stretch h-9 shrink-0">
            <VStat label="Budget" value={`${(vitals.util * 100).toFixed(0)}%`} />
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
            <VStat
              label="Pressure"
              value={vitals.pressure.toUpperCase()}
              tone={pressureTone(vitals.pressure)}
            />
          </div>
        )}

        {/* Filter / view bar */}
        <div className="flex items-center gap-2 flex-wrap shrink-0">
          {/* Live / History toggle */}
          <div className="flex items-stretch gap-0">
            {(
              [
                { key: "live" as const, label: "Live" },
                { key: "history" as const, label: "History" },
              ]
            ).map(({ key, label }) => (
              <button
                key={key}
                onClick={() => setView(key)}
                className={cn(
                  "border border-border-soft px-2.5 py-1 font-sans text-sm font-medium cursor-pointer transition-none -ml-[1px] first:ml-0",
                  view === key
                    ? "border-accent bg-surface-overlay text-text z-10 relative"
                    : "bg-surface text-text-muted hover:bg-surface-overlay hover:text-text",
                )}
              >
                {label}
                {key === "history" && history.length > 0 && (
                  <span className="ml-1 font-mono text-xs tabular-nums opacity-70">
                    {history.length}
                  </span>
                )}
              </button>
            ))}
          </div>

          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="filter by session or corpus"
            className="h-7 flex-1 max-w-xs border border-border-soft bg-surface px-2 text-sm font-sans text-text placeholder:text-text-dim focus:outline-none focus:border-accent transition-none"
          />
          <FilterPill
            active={pressureFilter === "all"}
            onClick={() => setPressureFilter("all")}
          >
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
          <FilterPill active={compact} onClick={() => setCompact((v) => !v)}>
            Compact
          </FilterPill>
          <span className="font-mono text-xs tabular-nums text-text-dim ml-auto">
            {filtered.length} / {sourceList.length}
          </span>
        </div>

        {/* Session list */}
        <section
          ref={focusRef}
          className="flex-1 min-h-0 overflow-y-auto"
        >
          {!loaded && view === "live" ? (
            <div className="font-serif text-base italic text-text-dim py-6">
              Loading<span className="ministr-blink">_</span>
            </div>
          ) : sourceList.length === 0 ? (
            view === "history" ? (
              <EmptyState
                icon={Users}
                title="No history yet"
                hint="Sessions that end while this page is open will appear here."
              />
            ) : (
              <SessionsEmpty />
            )
          ) : filtered.length === 0 ? (
            <EmptyState
              icon={Filter}
              title="No matches"
              hint="Clear the filter or widen the scope."
              action={
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    setQuery("");
                    setPressureFilter("all");
                  }}
                >
                  Reset
                </Button>
              }
            />
          ) : compact ? (
            <div className="border border-border-soft bg-surface">
              {filtered.map((sess) => (
                <CompactSessionRow
                  key={sess.session_id}
                  session={sess}
                  corpora={status.corpora}
                  fresh={freshSessions.has(sess.session_id)}
                  resolved={view === "history"}
                  onClick={() =>
                    openEntity({
                      kind: "session",
                      corpusId: sess.corpus_id,
                      sessionId: sess.session_id,
                      // Always pass the in-memory SessionDetail as a
                      // fallback. Required for history rows whose ids
                      // are no longer returned by `list_sessions`;
                      // harmless for live rows (live data takes priority).
                      seed: sess,
                    })
                  }
                />
              ))}
            </div>
          ) : (
            <div className="space-y-2">
              {filtered.map((sess) => (
                <div
                  key={sess.session_id}
                  className={cn(view === "history" && "opacity-70")}
                >
                  <TurnBlock
                    session={sess}
                    corpora={status.corpora}
                    fresh={freshSessions.has(sess.session_id)}
                    onClick={() =>
                      openEntity({
                        kind: "session",
                        corpusId: sess.corpus_id,
                        sessionId: sess.session_id,
                        // Same fallback as the compact-row case — keeps
                        // history-row sessions populated when list_sessions
                        // no longer returns them.
                        seed: sess,
                      })
                    }
                  />
                </div>
              ))}
            </div>
          )}
        </section>

        {/* Heartbeat dot */}
        <div className="flex items-center justify-end gap-2 shrink-0">
          <span className="font-mono text-xs uppercase tracking-[0.05em] text-text-dim">
            POLL
          </span>
          <span
            className={cn(
              "h-1.5 w-1.5",
              heartbeat ? "bg-accent" : "bg-border",
            )}
            aria-label="Polling heartbeat"
          />
        </div>
      </div>

      {/* RIGHT DRAWER — collapsible to 32px handle */}
      <aside
        className={cn(
          "hidden @min-[1024px]/page:flex shrink-0 transition-none border border-border-soft min-h-0",
          drawerOpen
            ? "w-[clamp(280px,30%,380px)] flex-col"
            : "w-8 flex-col items-stretch",
        )}
      >
        {drawerOpen ? (
          <>
            <div className="flex items-center justify-between border-b border-border-soft bg-surface-overlay px-3 py-2 shrink-0">
              <h3 className="font-serif text-base font-bold text-text">
                Observability
              </h3>
              <button
                onClick={() => setDrawerOpen(false)}
                aria-label="Collapse drawer"
                title="Collapse drawer"
                className="grid h-5 w-5 place-items-center border border-border-soft text-text-muted hover:text-text hover:border-border cursor-pointer transition-none"
                style={{ borderRadius: "var(--radius-button)" }}
              >
                <ChevronRight className="h-2.5 w-2.5" strokeWidth={2} />
              </button>
            </div>
            <section className="flex flex-col min-h-0 flex-1 overflow-hidden">
              <div className="flex items-baseline gap-3 border-b border-border-soft bg-surface-overlay px-3 py-1.5 shrink-0">
                <span className="font-serif text-sm font-normal text-text-dim tabular-nums shrink-0 w-5">
                  §1
                </span>
                <h4 className="font-serif text-sm font-bold text-text">
                  Activity
                </h4>
              </div>
              <div className="flex-1 min-h-0 overflow-y-auto">
                <ActivityFeed
                  events={activity}
                  limit={20}
                  flashSince={flashSince}
                />
              </div>
            </section>
            <section className="flex flex-col min-h-0 flex-1 overflow-hidden border-t border-border-soft">
              <div className="flex items-baseline gap-3 border-b border-border-soft bg-surface-overlay px-3 py-1.5 shrink-0">
                <span className="font-serif text-sm font-normal text-text-dim tabular-nums shrink-0 w-5">
                  §2
                </span>
                <h4 className="font-serif text-sm font-bold text-text">
                  Changes
                </h4>
              </div>
              <div className="flex-1 min-h-0 overflow-y-auto">
                <CoherenceFeed
                  events={coherence}
                  limit={12}
                  flashSince={flashSince}
                />
              </div>
            </section>
          </>
        ) : (
          <button
            onClick={() => setDrawerOpen(true)}
            aria-label="Expand drawer"
            title="Expand observability drawer"
            className="flex flex-col items-center justify-start gap-2 py-2 bg-surface text-text-muted hover:bg-surface-overlay hover:text-text cursor-pointer transition-none flex-1"
          >
            <ChevronLeft className="h-3 w-3" strokeWidth={2} />
            <span
              className="font-serif text-base font-normal"
              style={{ writingMode: "vertical-rl", textOrientation: "mixed" }}
            >
              Observability
            </span>
          </button>
        )}
      </aside>
    </div>
  );
}

// ─── COMPACT SESSION STRIP ─────────────────────────────────────────────────

function CompactSessionRow({
  session,
  corpora,
  fresh,
  resolved,
  onClick,
}: {
  session: SessionDetail;
  corpora: readonly CorpusInfo[];
  fresh?: boolean;
  resolved?: boolean;
  onClick?: () => void;
}) {
  const tone = pressureTone(session.pressure_level);
  const utilPct = (session.utilization * 100).toFixed(0);
  const sessionShort = session.session_id.slice(0, 8);
  return (
    <button
      onClick={onClick}
      disabled={!onClick}
      className={cn(
        "w-full text-left flex items-center gap-2 border-b-2 border-border last:border-b-0 px-2 py-1.5 transition-none",
        onClick && "cursor-pointer hover:bg-surface-overlay",
        fresh && "ministr-flash",
        resolved && "opacity-60",
      )}
    >
      <StatusDot tone={tone} pulse={fresh ? "live" : "off"} size="md" />
      <span className="font-mono text-[0.6875rem] text-text-muted shrink-0 w-20 truncate">
        {sessionShort}
      </span>
      <span className="font-mono text-xs text-text-dim shrink-0">
        T{session.current_turn}
      </span>
      <span
        className={cn(
          "font-mono text-xs font-bold uppercase tracking-[0.05em] w-16 shrink-0",
          toneTextClass(tone),
        )}
      >
        {session.pressure_level}
      </span>
      <div className="w-20 h-1.5 border border-border-soft bg-surface-overlay overflow-hidden shrink-0">
        <div
          className={cn(
            "h-full",
            session.pressure_level === "critical" && "bg-danger",
            session.pressure_level === "high" && "bg-warning",
            (session.pressure_level === "medium" ||
              session.pressure_level === "low" ||
              session.pressure_level === "none") &&
              "bg-accent",
          )}
          style={{ width: `${utilPct}%` }}
        />
      </div>
      <span className="font-mono text-xs tabular-nums w-10 text-right shrink-0">
        {utilPct}%
      </span>
      <span className="font-mono text-xs tabular-nums text-text-dim w-20 text-right shrink-0">
        {formatTokens(session.tokens_used)}
      </span>
      <span className="font-mono text-xs text-text-dim truncate flex-1">
        {corpusLabelById(corpora, session.corpus_id)}
      </span>
    </button>
  );
}

// ─── EMPTY STATE WITH MCP HINT ─────────────────────────────────────────────

function SessionsEmpty() {
  const cmd = "npx @modelcontextprotocol/inspector ministr stdio";
  return (
    <div className="border border-border-soft bg-surface p-8 text-center max-w-2xl mx-auto">
      <div className="grid h-12 w-12 mx-auto place-items-center border border-border-soft bg-surface-overlay text-text mb-4">
        <Users className="h-5 w-5" strokeWidth={2.5} />
      </div>
      <h3 className="font-serif text-2xl font-normal text-text leading-tight">
        No active sessions
      </h3>
      <p className="font-serif text-base italic text-text-dim mt-2 max-w-md mx-auto">
        Point Claude Code, Cursor, or any MCP client at the daemon — sessions
        appear here live with budget, pressure, and dedup metrics.
      </p>
      <div className="mt-5 text-left">
        <p className="font-sans text-sm font-semibold text-text-muted mb-1">
          How to start a session
        </p>
        <button
          onClick={() => navigator.clipboard.writeText(cmd)}
          title="Click to copy"
          className="block w-full text-left border border-border-soft bg-surface-sunken px-3 py-2 font-mono text-[0.8125rem] leading-[1.5] text-text break-all cursor-pointer hover:border-border hover:bg-surface-overlay transition-none"
        >
          {`> ${cmd}`}
        </button>
        <p className="font-serif text-xs italic text-text-dim mt-1">
          Click to copy.
        </p>
      </div>
    </div>
  );
}

// ─── PRIMITIVES ────────────────────────────────────────────────────────────

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
    <div className="flex items-center gap-2 px-3 py-1 border-r border-border-soft last:border-r-0 min-w-0">
      <span className="font-mono text-[0.6875rem] uppercase tracking-[0.05em] text-text-dim shrink-0">
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
        "border px-2 py-0.5 text-sm font-sans font-medium cursor-pointer transition-none",
        active
          ? "border-accent bg-surface-overlay text-text"
          : "border-border-soft bg-surface text-text-muted hover:border-border hover:text-text",
      )}
      style={{ borderRadius: "var(--radius-pill)" }}
    >
      {children}
    </button>
  );
}
