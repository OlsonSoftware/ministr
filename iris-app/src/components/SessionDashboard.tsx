import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Users,
  TrendingDown,
  Repeat,
  Copy,
  Gauge,
  Zap,
} from "lucide-react";
import { Card } from "./ui/card";
import { Badge } from "./ui/badge";
import { cn } from "../lib/utils";
import type { SessionDetail, DaemonStatus } from "../lib/types";

interface Props {
  status: DaemonStatus;
}

export function SessionDashboard({ status: _status }: Props) {
  const [sessions, setSessions] = useState<SessionDetail[]>([]);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    let cancelled = false;
    async function load() {
      try {
        const s = await invoke<SessionDetail[]>("list_sessions");
        if (!cancelled) {
          setSessions(s);
          setLoaded(true);
        }
      } catch {
        if (!cancelled) setLoaded(true);
      }
    }
    load();
    const interval = setInterval(load, 3000);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, []);

  const totalSessions = sessions.length;
  const totalTokens = sessions.reduce((s, x) => s + x.tokens_used, 0);
  const totalSaved = sessions.reduce((s, x) => s + x.total_tokens_saved, 0);
  const totalDedup = sessions.reduce((s, x) => s + x.dedup_hits, 0);

  return (
    <div className="space-y-4 iris-fade-in">
      <header className="flex items-end justify-between gap-4">
        <div>
          <h2 className="text-base font-semibold text-text">Sessions</h2>
          <p className="text-xs text-text-dim mt-0.5">
            Live view of every MCP agent attached to the daemon.
          </p>
        </div>
        {totalSessions > 0 && (
          <Badge variant="default" dot>
            {totalSessions} active
          </Badge>
        )}
      </header>

      {totalSessions > 0 && (
        <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
          <SummaryTile
            icon={Gauge}
            label="Tokens in flight"
            value={formatTokens(totalTokens)}
          />
          <SummaryTile
            icon={TrendingDown}
            label="Tokens saved"
            value={formatTokens(totalSaved)}
            tone="success"
          />
          <SummaryTile
            icon={Copy}
            label="Dedup hits"
            value={totalDedup.toLocaleString()}
            tone="accent"
          />
        </div>
      )}

      {!loaded ? (
        <div className="flex items-center justify-center py-12">
          <div className="iris-spin h-7 w-7 rounded-full border-2 border-border border-t-accent" />
        </div>
      ) : totalSessions === 0 ? (
        <EmptyState />
      ) : (
        <div className="grid gap-3">
          {sessions.map((s) => (
            <SessionCard key={s.session_id} session={s} />
          ))}
        </div>
      )}
    </div>
  );
}

function EmptyState() {
  return (
    <Card className="flex flex-col items-center justify-center gap-3 py-12 px-6 text-center">
      <div className="grid h-12 w-12 place-items-center rounded-xl bg-surface-overlay text-text-dim">
        <Users className="h-5 w-5" />
      </div>
      <div>
        <p className="text-sm font-medium text-text">No active sessions</p>
        <p className="text-xs text-text-dim mt-1 max-w-sm">
          Connect an MCP client (Claude Code, Cursor, or a custom agent) and
          sessions will stream in here with live budget, dedup, and compression
          metrics.
        </p>
      </div>
    </Card>
  );
}

function SummaryTile({
  icon: Icon,
  label,
  value,
  tone = "default",
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  value: string;
  tone?: "default" | "success" | "accent";
}) {
  return (
    <Card hover="lift" className="flex items-center gap-3">
      <div
        className={cn(
          "grid h-10 w-10 place-items-center rounded-lg shrink-0",
          tone === "success" && "bg-success/10 text-success",
          tone === "accent" && "bg-[var(--color-accent-soft)] text-accent",
          tone === "default" && "bg-surface-overlay text-text-muted",
        )}
      >
        <Icon className="h-4 w-4" />
      </div>
      <div className="min-w-0 flex-1">
        <p className="text-[11px] font-medium uppercase tracking-wider text-text-dim">
          {label}
        </p>
        <p className="text-lg font-semibold tabular-nums text-text leading-tight">
          {value}
        </p>
      </div>
    </Card>
  );
}

function SessionCard({ session: s }: { session: SessionDetail }) {
  const capacity = s.tokens_used + s.tokens_remaining;
  return (
    <Card hover="lift" className="space-y-3">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="font-mono text-[11px] text-text-muted truncate max-w-[260px]">
              {s.session_id}
            </span>
            <PressureBadge level={s.pressure_level} />
          </div>
          <p className="text-[11px] text-text-dim mt-1">
            <span className="font-mono">{s.corpus_id}</span> · Turn{" "}
            <span className="tabular-nums">{s.current_turn}</span> ·{" "}
            {s.delivered_count} delivered
          </p>
        </div>
      </div>

      <div>
        <div className="flex justify-between text-[11px] mb-1.5">
          <span className="flex items-center gap-1.5 text-text-muted">
            <Gauge className="h-3 w-3" />
            Budget
          </span>
          <span className="font-mono tabular-nums text-text">
            {formatTokens(s.tokens_used)} / {formatTokens(capacity)}
            <span className="text-text-dim ml-1.5">
              ({(s.utilization * 100).toFixed(0)}%)
            </span>
          </span>
        </div>
        <BudgetBar utilization={s.utilization} pressure={s.pressure_level} />
      </div>

      {s.total_deliveries > 0 && (
        <div className="grid grid-cols-3 gap-2 pt-3 border-t border-border/60">
          <StatBlock
            icon={TrendingDown}
            value={formatTokens(s.total_tokens_saved)}
            label="saved"
            tone="success"
          />
          <StatBlock
            icon={Repeat}
            value={`${(s.compression_ratio * 100).toFixed(0)}%`}
            label="compression"
            tone="accent"
          />
          <StatBlock
            icon={Copy}
            value={s.dedup_hits.toString()}
            label="dedup hits"
            tone="info"
          />
        </div>
      )}
    </Card>
  );
}

function StatBlock({
  icon: Icon,
  value,
  label,
  tone,
}: {
  icon: React.ComponentType<{ className?: string }>;
  value: string;
  label: string;
  tone: "success" | "accent" | "info";
}) {
  const toneClass =
    tone === "success"
      ? "text-success"
      : tone === "accent"
        ? "text-accent"
        : "text-text";
  return (
    <div className="text-center">
      <div className={cn("flex items-center justify-center gap-1 mb-0.5", toneClass)}>
        <Icon className="h-3 w-3" />
        <span className="text-sm font-semibold tabular-nums">{value}</span>
      </div>
      <span className="text-[10px] text-text-dim uppercase tracking-wider">
        {label}
      </span>
    </div>
  );
}

function BudgetBar({
  utilization,
  pressure,
}: {
  utilization: number;
  pressure: string;
}) {
  const pct = Math.min(utilization * 100, 100);
  const [from, to] =
    pressure === "critical"
      ? ["from-danger", "to-danger"]
      : pressure === "high"
        ? ["from-warning", "to-warning"]
        : pressure === "medium"
          ? ["from-accent", "to-[color-mix(in_srgb,var(--color-accent)_60%,#c4b5fd)]"]
          : ["from-success", "to-success"];

  return (
    <div className="relative h-1.5 overflow-hidden rounded-full bg-surface-overlay">
      <div
        className={cn(
          "h-full rounded-full transition-all duration-300 bg-gradient-to-r",
          from,
          to,
        )}
        style={{ width: `${pct}%` }}
      />
    </div>
  );
}

function PressureBadge({ level }: { level: string }) {
  const variant: Record<string, "success" | "default" | "warning" | "danger" | "muted"> = {
    none: "success",
    low: "success",
    medium: "default",
    high: "warning",
    critical: "danger",
  };
  const icon = level === "critical" ? Zap : undefined;
  return (
    <Badge
      variant={variant[level] ?? "muted"}
      dot={level === "critical" || level === "high"}
    >
      {icon && <Zap className="h-2.5 w-2.5" />}
      {level}
    </Badge>
  );
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toString();
}
