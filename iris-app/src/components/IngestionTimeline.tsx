import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Timer, HardDrive, Layers } from "lucide-react";
import { Card } from "./ui/card";
import type { DaemonStatus, IngestionProgressInfo } from "../lib/types";

interface Props {
  status: DaemonStatus;
}

export function IngestionTimeline({ status }: Props) {
  const [progress, setProgress] = useState<IngestionProgressInfo[]>([]);

  useEffect(() => {
    let cancelled = false;
    async function poll() {
      try {
        const all: IngestionProgressInfo[] = [];
        for (const c of status.corpora) {
          const p = await invoke<IngestionProgressInfo>("ingestion_progress", {
            corpusId: c.id,
          });
          all.push(p);
        }
        if (!cancelled) setProgress(all);
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
            const pct = p.files_total > 0 ? (p.files_done / p.files_total) * 100 : 100;
            const isActive = p.files_total > 0 && p.files_done < p.files_total;

            return (
              <Card key={p.corpus_id}>
                <div className="flex items-center justify-between mb-2">
                  <span className="font-mono text-xs truncate max-w-[250px]">
                    {p.corpus_id}
                  </span>
                  {isActive ? (
                    <span className="text-xs px-2 py-0.5 rounded-full bg-warning/10 text-warning animate-pulse">
                      indexing
                    </span>
                  ) : (
                    <span className="text-xs px-2 py-0.5 rounded-full bg-green-500/10 text-green-500">
                      idle
                    </span>
                  )}
                </div>

                <div className="h-2 rounded-full bg-surface-overlay overflow-hidden mb-2">
                  <div
                    className="h-full rounded-full bg-accent transition-all"
                    style={{ width: `${pct}%` }}
                  />
                </div>

                <div className="grid grid-cols-3 gap-2 text-xs text-text-dim">
                  <div className="flex items-center gap-1">
                    <HardDrive className="h-3 w-3" />
                    {p.files_done}/{p.files_total} files
                  </div>
                  <div className="flex items-center gap-1">
                    <Layers className="h-3 w-3" />
                    {corpusInfo?.sections_count.toLocaleString() ?? "?"} sections
                  </div>
                  <div className="flex items-center gap-1">
                    <span className="text-accent">⚡</span>
                    {p.embeddings_done.toLocaleString()} embeddings
                  </div>
                </div>
              </Card>
            );
          })}
        </div>
      )}
    </div>
  );
}
