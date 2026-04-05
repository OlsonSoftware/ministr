import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Timer, HardDrive, Layers, Cpu, FileText } from "lucide-react";
import { Card } from "./ui/card";
import type { DaemonStatus, IngestionProgressInfo } from "../lib/types";

interface Props {
  status: DaemonStatus;
}

const PHASE_LABELS: Record<string, string> = {
  idle: "Idle",
  discovering: "Discovering files…",
  parsing: "Parsing & extracting…",
  embedding: "Generating embeddings…",
  finalizing: "Finalizing…",
};

function phaseLabel(phase: string): string {
  return PHASE_LABELS[phase] ?? phase;
}

function ProgressBar({ value, max, className = "" }: { value: number; max: number; className?: string }) {
  const pct = max > 0 ? Math.min((value / max) * 100, 100) : 0;
  return (
    <div className={`h-1.5 rounded-full bg-surface-overlay overflow-hidden ${className}`}>
      <div
        className="h-full rounded-full transition-all duration-300"
        style={{ width: `${pct}%` }}
      />
    </div>
  );
}

export function IngestionTimeline({ status }: Props) {
  const [progress, setProgress] = useState<IngestionProgressInfo[]>([]);

  useEffect(() => {
    let cancelled = false;
    async function poll() {
      try {
        const result = await invoke<IngestionProgressInfo[]>("ingestion_progress");
        if (!cancelled) setProgress(result);
      } catch {
        /* ignore */
      }
    }
    poll();
    const interval = setInterval(poll, 1000);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [status.corpora]);

  return (
    <div className="space-y-3">
      <h2 className="text-sm font-medium text-text-muted uppercase tracking-wider flex items-center gap-2">
        <Timer className="h-4 w-4" /> Ingestion Progress
      </h2>

      {progress.length === 0 ? (
        <p className="text-sm text-text-dim">No corpora registered.</p>
      ) : (
        <div className="grid gap-3">
          {progress.map((p) => {
            const corpusInfo = status.corpora.find((c) => c.id === p.corpus_id);
            const isActive = p.status === 1;
            const isComplete = p.status === 2;

            return (
              <Card key={p.corpus_id}>
                {/* Header: corpus ID + status badge */}
                <div className="flex items-center justify-between mb-2">
                  <span className="font-mono text-xs truncate max-w-[250px]">
                    {p.corpus_id}
                  </span>
                  {isActive ? (
                    <span className="text-xs px-2 py-0.5 rounded-full bg-warning/10 text-warning animate-pulse">
                      {phaseLabel(p.phase)}
                    </span>
                  ) : isComplete ? (
                    <span className="text-xs px-2 py-0.5 rounded-full bg-green-500/10 text-green-500">
                      complete
                    </span>
                  ) : (
                    <span className="text-xs px-2 py-0.5 rounded-full bg-surface-overlay text-text-dim">
                      pending
                    </span>
                  )}
                </div>

                {/* Files progress bar */}
                <div className="space-y-1 mb-2">
                  <div className="flex items-center justify-between text-xs text-text-dim">
                    <span className="flex items-center gap-1">
                      <HardDrive className="h-3 w-3" />
                      Files
                    </span>
                    <span>{p.files_done}/{p.files_total}</span>
                  </div>
                  <ProgressBar value={p.files_done} max={p.files_total} className="[&>div]:bg-accent" />
                </div>

                {/* Embeddings progress bar (only shown when there's embedding work) */}
                {(p.embeddings_total > 0 || (isActive && p.phase === "embedding")) && (
                  <div className="space-y-1 mb-2">
                    <div className="flex items-center justify-between text-xs text-text-dim">
                      <span className="flex items-center gap-1">
                        <Cpu className="h-3 w-3" />
                        Embeddings
                      </span>
                      <span>{p.embeddings_done.toLocaleString()}/{p.embeddings_total.toLocaleString()}</span>
                    </div>
                    <ProgressBar value={p.embeddings_done} max={p.embeddings_total} className="[&>div]:bg-purple-500" />
                  </div>
                )}

                {/* Stats row */}
                <div className="grid grid-cols-3 gap-2 text-xs text-text-dim">
                  <div className="flex items-center gap-1">
                    <Layers className="h-3 w-3" />
                    {p.sections_done.toLocaleString()} sections
                  </div>
                  <div className="flex items-center gap-1">
                    <span className="text-accent">⚡</span>
                    {p.embeddings_done.toLocaleString()} vectors
                  </div>
                  <div className="flex items-center gap-1 justify-end">
                    {corpusInfo?.sections_count !== undefined && (
                      <span>
                        {corpusInfo.sections_count.toLocaleString()} total
                      </span>
                    )}
                  </div>
                </div>

                {/* Current file indicator */}
                {isActive && p.current_file && (
                  <div className="mt-2 flex items-center gap-1.5 text-xs text-text-dim truncate">
                    <FileText className="h-3 w-3 flex-shrink-0 text-accent" />
                    <span className="truncate font-mono opacity-70">{p.current_file}</span>
                  </div>
                )}
              </Card>
            );
          })}
        </div>
      )}
    </div>
  );
}
