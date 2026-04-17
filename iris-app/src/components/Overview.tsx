import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity,
  Database,
  Sparkles,
  Zap,
  RefreshCw,
  AlertTriangle,
  Radio,
  Waves,
  FolderKanban,
  Plus,
  Eye,
} from "lucide-react";
import type {
  DaemonStatus,
  SessionDetail,
  IngestionProgressInfo,
} from "../lib/types";
import { Card } from "./ui/card";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { BudgetRing } from "./ui/budget-ring";
import { CorpusChip } from "./ui/corpus-chip";
import { StatusDot } from "./ui/status-dot";
import { TurnBlock } from "./ui/turn-block";
import { cn } from "../lib/utils";

interface OverviewProps {
  status: DaemonStatus;
  selectedCorpusId: string | null;
  onSelectCorpus: (id: string | null) => void;
  onOpenProjects: () => void;
  onOpenSessions: () => void;
  onAddProject: () => void;
  onRefresh: () => void;
}

export function Overview({
  status,
  selectedCorpusId,
  onSelectCorpus,
  onOpenProjects,
  onOpenSessions,
  onAddProject,
  onRefresh,
}: OverviewProps) {
  const [sessions, setSessions] = useState<SessionDetail[]>([]);
  const [ingestion, setIngestion] = useState<IngestionProgressInfo[]>([]);
  const [freshSessions, setFreshSessions] = useState<Set<string>>(new Set());
  const prevTurns = useRef<Map<string, number>>(new Map());

  useEffect(() => {
    let cancelled = false;
    async function poll() {
      try {
        const [s, ing] = await Promise.all([
          invoke<SessionDetail[]>("list_sessions"),
          invoke<IngestionProgressInfo[]>("ingestion_progress"),
        ]);
        if (cancelled) return;
        setIngestion(ing);

        // Detect new turns → mark the session as fresh so TurnBlock flashes.
        const fresh = new Set<string>();
        for (const sess of s) {
          const prev = prevTurns.current.get(sess.session_id);
          if (prev !== undefined && sess.current_turn > prev) {
            fresh.add(sess.session_id);
          }
          prevTurns.current.set(sess.session_id, sess.current_turn);
        }
        setSessions(s);
        if (fresh.size) {
          setFreshSessions(fresh);
          setTimeout(() => {
            if (!cancelled) setFreshSessions(new Set());
          }, 1500);
        }
      } catch {
        /* ignore */
      }
    }
    poll();
    const id = setInterval(poll, 1500);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, []);

  // Vitals aggregates
  const vitals = useMemo(() => {
    const sessionCount = sessions.length;
    const totalTokensUsed = sessions.reduce((s, x) => s + x.tokens_used, 0);
    const totalCapacity = sessions.reduce(
      (s, x) => s + x.tokens_used + x.tokens_remaining,
      0,
    );
    const util = totalCapacity > 0 ? totalTokensUsed / totalCapacity : 0;
    const totalDelivered = sessions.reduce(
      (s, x) => s + x.total_deliveries,
      0,
    );
    const totalDedup = sessions.reduce((s, x) => s + x.dedup_hits, 0);
    const hitRate = totalDelivered > 0 ? totalDedup / totalDelivered : 0;
    const totalSaved = sessions.reduce((s, x) => s + x.total_tokens_saved, 0);

    const maxPressure = sessions.reduce<
      "none" | "low" | "medium" | "high" | "critical"
    >((max, x) => {
      const order = ["none", "low", "medium", "high", "critical"] as const;
      return order.indexOf(x.pressure_level as (typeof order)[number]) >
        order.indexOf(max)
        ? (x.pressure_level as (typeof order)[number])
        : max;
    }, "none");

    const totalFiles = status.corpora.reduce((s, c) => s + c.files_indexed, 0);
    const totalSections = status.corpora.reduce(
      (s, c) => s + c.sections_count,
      0,
    );
    const totalVectors = status.corpora.reduce(
      (s, c) => s + c.embeddings_count,
      0,
    );
    const indexingCount = status.corpora.filter(
      (c) => c.status.state === "indexing",
    ).length;

    return {
      sessionCount,
      totalTokensUsed,
      totalCapacity,
      util,
      totalDelivered,
      totalDedup,
      hitRate,
      totalSaved,
      maxPressure,
      totalFiles,
      totalSections,
      totalVectors,
      indexingCount,
    };
  }, [sessions, status.corpora]);

  const activeIngestion = ingestion.filter((p) => p.status === 1);

  return (
    <div className="space-y-5 iris-fade-in">
      {/* Page title */}
      <header className="flex items-end justify-between gap-4">
        <div>
          <h1 className="text-lg font-semibold text-text flex items-center gap-2">
            <Radio className="h-4 w-4 text-accent" />
            Cache observatory
          </h1>
          <p className="text-xs text-text-dim mt-0.5">
            Live telemetry for the iris context cache —{" "}
            <span className="font-mono">
              {status.corpora.length} corpora · {vitals.sessionCount} sessions
            </span>
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" onClick={onRefresh}>
            <RefreshCw className="h-3.5 w-3.5" />
            Refresh
          </Button>
          <Button size="sm" onClick={onAddProject}>
            <Plus className="h-3.5 w-3.5" />
            Add project
          </Button>
        </div>
      </header>

      {/* Vitals row */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
        <VitalCard
          title="Aggregate budget"
          subtitle="Tokens used across active sessions"
          empty={vitals.sessionCount === 0}
          emptyLabel="No active sessions"
        >
          <BudgetRing
            utilization={vitals.util}
            pressure={vitals.maxPressure}
            size={118}
            stroke={8}
          >
            <span className="font-mono text-2xl font-bold tabular-nums text-text leading-none">
              {(vitals.util * 100).toFixed(0)}
              <span className="text-sm text-text-dim">%</span>
            </span>
            <span className="text-[10px] uppercase tracking-wider text-text-dim mt-1">
              {formatTokens(vitals.totalTokensUsed)} /{" "}
              {formatTokens(vitals.totalCapacity)}
            </span>
          </BudgetRing>
        </VitalCard>

        <VitalCard
          title="Cache hit rate"
          subtitle="Dedup hits vs total deliveries"
          empty={vitals.totalDelivered === 0}
          emptyLabel="No deliveries yet"
          right={
            <div className="text-right">
              <div className="text-[10px] uppercase tracking-wider text-text-dim">
                saved
              </div>
              <div className="font-mono text-sm font-semibold text-success tabular-nums">
                {formatTokens(vitals.totalSaved)}
              </div>
            </div>
          }
        >
          <div className="flex items-center gap-4">
            <div className="flex flex-col">
              <span className="font-mono text-3xl font-bold tabular-nums text-text leading-none">
                {(vitals.hitRate * 100).toFixed(0)}
                <span className="text-base text-text-dim">%</span>
              </span>
              <span className="text-[10px] uppercase tracking-wider text-text-dim mt-1.5">
                {vitals.totalDedup} hits · {vitals.totalDelivered} total
              </span>
            </div>
            <HitRateBars rate={vitals.hitRate} />
          </div>
        </VitalCard>

        <VitalCard
          title="Index"
          subtitle={`Across ${status.corpora.length} ${status.corpora.length === 1 ? "corpus" : "corpora"}`}
          right={
            vitals.indexingCount > 0 ? (
              <Badge variant="warning" dot>
                {vitals.indexingCount} indexing
              </Badge>
            ) : undefined
          }
        >
          <div className="grid grid-cols-3 gap-2">
            <StatCell label="Files" value={vitals.totalFiles} />
            <StatCell label="Sections" value={vitals.totalSections} />
            <StatCell label="Vectors" value={vitals.totalVectors} />
          </div>
          {activeIngestion.length > 0 && (
            <div className="mt-3 space-y-1.5">
              {activeIngestion.slice(0, 2).map((p) => (
                <IngestionTicker key={p.corpus_id} progress={p} />
              ))}
            </div>
          )}
        </VitalCard>
      </div>

      {/* Corpus strip */}
      <section>
        <div className="flex items-center justify-between mb-2">
          <h2 className="text-[11px] font-semibold uppercase tracking-wider text-text-dim flex items-center gap-1.5">
            <FolderKanban className="h-3 w-3" />
            Corpora
          </h2>
          <button
            onClick={onOpenProjects}
            className="text-[11px] text-text-dim hover:text-text cursor-pointer"
          >
            Manage all →
          </button>
        </div>
        {status.corpora.length === 0 ? (
          <Card className="flex flex-col items-center gap-2 py-8 text-center">
            <div className="grid h-10 w-10 place-items-center rounded-lg bg-[var(--color-accent-soft)] text-accent">
              <FolderKanban className="h-5 w-5" />
            </div>
            <p className="text-sm font-medium text-text">No corpora yet</p>
            <p className="text-xs text-text-dim max-w-xs">
              Add a project directory to start indexing and serving context to
              agents.
            </p>
            <Button size="sm" onClick={onAddProject} className="mt-1">
              <Plus className="h-3.5 w-3.5" />
              Add your first project
            </Button>
          </Card>
        ) : (
          <div className="flex gap-2 overflow-x-auto pb-1 -mx-1 px-1">
            {status.corpora.map((c) => (
              <CorpusChip
                key={c.id}
                corpus={c}
                selected={selectedCorpusId === c.id}
                onClick={() =>
                  onSelectCorpus(selectedCorpusId === c.id ? null : c.id)
                }
              />
            ))}
          </div>
        )}
      </section>

      {/* Live stream + side panels */}
      <div className="grid grid-cols-1 lg:grid-cols-[3fr_2fr] gap-3">
        {/* Live session stream */}
        <section>
          <div className="flex items-center justify-between mb-2">
            <h2 className="text-[11px] font-semibold uppercase tracking-wider text-text-dim flex items-center gap-1.5">
              <Waves className="h-3 w-3" />
              Live turn stream
              {sessions.length > 0 && (
                <span className="inline-flex items-center gap-1 ml-2 text-[10px] text-accent normal-case font-sans font-medium">
                  <StatusDot tone="accent" pulse />
                  streaming
                </span>
              )}
            </h2>
            <button
              onClick={onOpenSessions}
              className="text-[11px] text-text-dim hover:text-text cursor-pointer"
            >
              Session detail →
            </button>
          </div>

          {sessions.length === 0 ? (
            <Card className="flex flex-col items-center gap-3 py-10 text-center">
              <div className="grid h-12 w-12 place-items-center rounded-xl bg-surface-overlay text-text-dim">
                <Radio className="h-5 w-5" />
              </div>
              <div className="space-y-1">
                <p className="text-sm font-medium text-text">
                  Nothing streaming yet
                </p>
                <p className="max-w-sm text-xs text-text-dim">
                  Connect Claude Code, Cursor, or any MCP client pointed at{" "}
                  <span className="font-mono">~/.iris/irisd.sock</span> — tool
                  calls will stream here in real time.
                </p>
              </div>
            </Card>
          ) : (
            <div className="space-y-2">
              {sessions.map((s) => (
                <TurnBlock
                  key={s.session_id}
                  session={s}
                  fresh={freshSessions.has(s.session_id)}
                  onClick={onOpenSessions}
                />
              ))}
            </div>
          )}
        </section>

        {/* Side panels: prefetch lookahead + coherence */}
        <section className="space-y-3">
          <SidePanel
            icon={Sparkles}
            title="Prefetch lookahead"
            note="requires iris 0.2"
          >
            <p className="text-xs text-text-dim leading-relaxed">
              The prefetch engine runs inside the daemon and pre-warms the cache
              using sequential, topical, structural, and cross-session
              locality. A live view of its predictions lands in a coming
              release.
            </p>
          </SidePanel>

          <SidePanel
            icon={Activity}
            title="Coherence feed"
            note="requires iris 0.2"
          >
            <p className="text-xs text-text-dim leading-relaxed">
              When files change on disk, the coherence engine re-parses,
              re-embeds, and marks affected sections stale. File-change events
              will stream here.
            </p>
          </SidePanel>

          <SidePanel icon={Database} title="Daemon">
            <dl className="space-y-1.5 text-xs">
              <Row label="Model" value={status.model} mono />
              <Row
                label="Dimension"
                value={`${status.model_dimension}d`}
                mono
              />
              <Row label="RSS" value={`${status.memory_mb.toFixed(0)} MB`} mono />
              <Row label="Version" value={`v${status.version}`} mono />
            </dl>
          </SidePanel>
        </section>
      </div>
    </div>
  );
}

function VitalCard({
  title,
  subtitle,
  children,
  empty,
  emptyLabel,
  right,
}: {
  title: string;
  subtitle?: string;
  children: React.ReactNode;
  empty?: boolean;
  emptyLabel?: string;
  right?: React.ReactNode;
}) {
  return (
    <Card hover="lift" className="p-4">
      <div className="flex items-start justify-between gap-2 mb-3">
        <div>
          <h3 className="text-[11px] font-semibold uppercase tracking-wider text-text-dim">
            {title}
          </h3>
          {subtitle && (
            <p className="text-[11px] text-text-dim mt-0.5">{subtitle}</p>
          )}
        </div>
        {right}
      </div>
      {empty ? (
        <div className="flex h-[118px] items-center justify-center">
          <span className="text-xs text-text-dim">{emptyLabel}</span>
        </div>
      ) : (
        children
      )}
    </Card>
  );
}

function SidePanel({
  icon: Icon,
  title,
  note,
  children,
}: {
  icon: React.ComponentType<{ className?: string }>;
  title: string;
  note?: string;
  children: React.ReactNode;
}) {
  return (
    <Card hover="lift" className="p-3">
      <div className="flex items-center gap-1.5 mb-2">
        <Icon className="h-3.5 w-3.5 text-text-dim" />
        <h3 className="text-[11px] font-semibold uppercase tracking-wider text-text-dim flex-1">
          {title}
        </h3>
        {note && (
          <Badge variant="muted" className="text-[9px]">
            {note}
          </Badge>
        )}
      </div>
      {children}
    </Card>
  );
}

function HitRateBars({ rate }: { rate: number }) {
  // 12 bars representing buckets — not real history yet; the shape conveys
  // the general idea. Each bar height scales with the rate so the shape
  // changes as hit rate evolves.
  const bars = Array.from({ length: 12 }, (_, i) => {
    const noise = (Math.sin(i * 2.3) + 1) * 0.15;
    const h = Math.max(0.1, Math.min(1, rate + noise - 0.2));
    return h;
  });
  return (
    <div className="flex items-end gap-0.5 h-12">
      {bars.map((h, i) => (
        <div
          key={i}
          className="w-1 rounded-full bg-gradient-to-t from-accent/40 to-accent"
          style={{ height: `${h * 100}%` }}
        />
      ))}
    </div>
  );
}

function StatCell({ label, value }: { label: string; value: number }) {
  return (
    <div className="flex flex-col">
      <span className="text-[10px] uppercase tracking-wider text-text-dim">
        {label}
      </span>
      <span className="font-mono text-lg font-semibold tabular-nums text-text leading-tight">
        {value.toLocaleString()}
      </span>
    </div>
  );
}

function IngestionTicker({ progress }: { progress: IngestionProgressInfo }) {
  const pct =
    progress.files_total > 0
      ? (progress.files_done / progress.files_total) * 100
      : 0;
  const label = progress.corpus_id.slice(0, 12);
  return (
    <div className="text-[10px] font-mono">
      <div className="flex items-center justify-between">
        <span className="text-text-muted truncate">{label}</span>
        <span className="text-warning tabular-nums">{pct.toFixed(0)}%</span>
      </div>
      <div className="mt-0.5 h-0.5 rounded-full bg-surface-overlay overflow-hidden">
        <div
          className="h-full bg-warning"
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  );
}

function Row({
  label,
  value,
  mono,
}: {
  label: string;
  value: React.ReactNode;
  mono?: boolean;
}) {
  return (
    <div className="flex items-center justify-between">
      <dt className="text-text-dim">{label}</dt>
      <dd className={cn("text-text", mono && "font-mono tabular-nums")}>
        {value}
      </dd>
    </div>
  );
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toString();
}

// Hide unused imports warnings on exports.
export const __previewIcons = { Zap, Eye };
