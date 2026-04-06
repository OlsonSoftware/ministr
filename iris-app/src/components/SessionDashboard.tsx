import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Users, TrendingDown, Repeat, Copy } from "lucide-react";
import { Card } from "./ui/card";
import type { SessionDetail, DaemonStatus } from "../lib/types";

interface Props {
  status: DaemonStatus;
}

export function SessionDashboard({ status }: Props) {
  const [sessions, setSessions] = useState<SessionDetail[]>([]);

  useEffect(() => {
    let cancelled = false;
    async function load() {
      try {
        const s = await invoke<SessionDetail[]>("list_sessions");
        if (!cancelled) setSessions(s);
      } catch {
        /* ignore */
      }
    }
    load();
    const interval = setInterval(load, 3000);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, []);

  return (
    <div className="space-y-3">
      <h2 className="text-sm font-medium text-text-muted uppercase tracking-wider flex items-center gap-2">
        <Users className="h-4 w-4" /> Active Sessions
      </h2>

      {sessions.length === 0 ? (
        <p className="text-sm text-text-dim">No active sessions.</p>
      ) : (
        <div className="grid gap-3">
          {sessions.map((s) => (
            <Card key={s.session_id}>
              <div className="flex items-center justify-between mb-2">
                <span className="text-xs font-mono text-text-dim truncate max-w-[200px]">
                  {s.session_id}
                </span>
                <PressureBadge level={s.pressure_level} />
              </div>

              <div className="flex items-center gap-4 text-xs text-text-dim mb-2">
                <span>Corpus: {s.corpus_id}</span>
                <span>Turn {s.current_turn}</span>
                <span>{s.delivered_count} delivered</span>
              </div>

              <div className="space-y-1">
                <div className="flex justify-between text-xs">
                  <span>Token budget</span>
                  <span>
                    {formatTokens(s.tokens_used)} / {formatTokens(s.tokens_used + s.tokens_remaining)}
                  </span>
                </div>
                <BudgetBar utilization={s.utilization} pressure={s.pressure_level} />
              </div>

              {s.total_deliveries > 0 && (
                <div className="grid grid-cols-3 gap-2 mt-3 pt-3 border-t border-surface-overlay">
                  <div className="text-center">
                    <div className="flex items-center justify-center gap-1 text-green-500 mb-0.5">
                      <TrendingDown className="h-3 w-3" />
                      <span className="text-xs font-medium">{formatTokens(s.total_tokens_saved)}</span>
                    </div>
                    <span className="text-[10px] text-text-dim">saved</span>
                  </div>
                  <div className="text-center">
                    <div className="flex items-center justify-center gap-1 text-accent mb-0.5">
                      <Repeat className="h-3 w-3" />
                      <span className="text-xs font-medium">{(s.compression_ratio * 100).toFixed(0)}%</span>
                    </div>
                    <span className="text-[10px] text-text-dim">compression</span>
                  </div>
                  <div className="text-center">
                    <div className="flex items-center justify-center gap-1 text-blue-400 mb-0.5">
                      <Copy className="h-3 w-3" />
                      <span className="text-xs font-medium">{s.dedup_hits}</span>
                    </div>
                    <span className="text-[10px] text-text-dim">dedup hits</span>
                  </div>
                </div>
              )}
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}

function BudgetBar({ utilization, pressure }: { utilization: number; pressure: string }) {
  const pct = Math.min(utilization * 100, 100);
  const color =
    pressure === "critical"
      ? "bg-danger"
      : pressure === "high"
        ? "bg-warning"
        : pressure === "medium"
          ? "bg-accent"
          : "bg-green-500";

  return (
    <div className="h-2 rounded-full bg-surface-overlay overflow-hidden">
      <div className={`h-full rounded-full transition-all ${color}`} style={{ width: `${pct}%` }} />
    </div>
  );
}

function PressureBadge({ level }: { level: string }) {
  const colors: Record<string, string> = {
    none: "bg-green-500/10 text-green-500",
    low: "bg-green-500/10 text-green-500",
    medium: "bg-accent/10 text-accent",
    high: "bg-warning/10 text-warning",
    critical: "bg-danger/10 text-danger",
  };
  return (
    <span className={`text-xs px-2 py-0.5 rounded-full ${colors[level] ?? colors.low}`}>
      {level}
    </span>
  );
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toString();
}
