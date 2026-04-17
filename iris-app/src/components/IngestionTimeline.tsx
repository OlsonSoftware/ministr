import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Timer,
  HardDrive,
  Layers,
  Cpu,
  FileText,
  CheckCircle2,
  Loader2,
  Database,
} from "lucide-react";
import { Card } from "./ui/card";
import { Badge } from "./ui/badge";
import { Progress } from "./ui/progress";
import { cn } from "../lib/utils";
import type { DaemonStatus, IngestionProgressInfo } from "../lib/types";

interface Props {
  status: DaemonStatus;
}

const PHASE_LABELS: Record<string, string> = {
  idle: "Idle",
  discovering: "Discovering files",
  parsing: "Parsing & extracting",
  embedding: "Generating embeddings",
  finalizing: "Finalizing",
};

function phaseLabel(phase: string): string {
  return PHASE_LABELS[phase] ?? phase;
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

  const activeCount = progress.filter((p) => p.status === 1).length;

  return (
    <div className="space-y-4 iris-fade-in">
      <header className="flex items-end justify-between gap-4">
        <div>
          <h2 className="text-base font-semibold text-text">Ingestion</h2>
          <p className="text-xs text-text-dim mt-0.5">
            Live parse / embed / index progress per corpus.
          </p>
        </div>
        {activeCount > 0 && (
          <Badge variant="warning" dot>
            {activeCount} indexing
          </Badge>
        )}
      </header>

      {progress.length === 0 ? (
        <Card className="flex flex-col items-center justify-center gap-2 py-10 text-center">
          <div className="grid h-12 w-12 place-items-center rounded-xl bg-surface-overlay text-text-dim">
            <Timer className="h-5 w-5" />
          </div>
          <p className="text-sm font-medium text-text">No corpora registered</p>
          <p className="text-xs text-text-dim">
            Add a project to see ingestion progress stream in.
          </p>
        </Card>
      ) : (
        <div className="grid gap-3">
          {progress.map((p) => {
            const corpusInfo = status.corpora.find((c) => c.id === p.corpus_id);
            return (
              <IngestionCard
                key={p.corpus_id}
                progress={p}
                totalSections={corpusInfo?.sections_count}
              />
            );
          })}
        </div>
      )}
    </div>
  );
}

function IngestionCard({
  progress: p,
  totalSections,
}: {
  progress: IngestionProgressInfo;
  totalSections?: number;
}) {
  const isActive = p.status === 1;
  const isComplete = p.status === 2;

  return (
    <Card hover="lift" className="space-y-3">
      <header className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="font-mono text-[11px] text-text-muted truncate max-w-[280px]">
              {p.corpus_id}
            </span>
            {isActive ? (
              <Badge variant="warning" dot>
                <Loader2 className="h-2.5 w-2.5 iris-spin" />
                {phaseLabel(p.phase)}
              </Badge>
            ) : isComplete ? (
              <Badge variant="success">
                <CheckCircle2 className="h-2.5 w-2.5" />
                Complete
              </Badge>
            ) : (
              <Badge variant="muted">Pending</Badge>
            )}
          </div>
        </div>
      </header>

      <ProgressRow
        icon={HardDrive}
        label="Files"
        done={p.files_done}
        total={p.files_total}
      />

      {(p.embeddings_total > 0 || (isActive && p.phase === "embedding")) && (
        <ProgressRow
          icon={Cpu}
          label="Embeddings"
          done={p.embeddings_done}
          total={p.embeddings_total}
          glow
        />
      )}

      <div className="grid grid-cols-3 gap-2 pt-2 border-t border-border/60">
        <StatCell icon={Layers} value={p.sections_done} label="sections" />
        <StatCell icon={Database} value={p.embeddings_done} label="vectors" />
        <StatCell
          icon={Layers}
          value={totalSections}
          label="total indexed"
          muted
        />
      </div>

      {isActive && p.current_file && (
        <div className="flex items-center gap-1.5 text-[11px] text-text-muted bg-surface-sunken border border-border/60 rounded-md px-2.5 py-1.5 truncate">
          <FileText className="h-3 w-3 shrink-0 text-accent" />
          <span className="truncate font-mono">{p.current_file}</span>
        </div>
      )}
    </Card>
  );
}

function ProgressRow({
  icon: Icon,
  label,
  done,
  total,
  glow = false,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  done: number;
  total: number;
  glow?: boolean;
}) {
  const pct = total > 0 ? (done / total) * 100 : 0;
  return (
    <div>
      <div className="flex items-center justify-between text-[11px] mb-1.5">
        <span className="flex items-center gap-1.5 text-text-muted">
          <Icon className="h-3 w-3" />
          {label}
        </span>
        <span className="font-mono tabular-nums text-text">
          {done.toLocaleString()} / {total.toLocaleString()}
          <span className="text-text-dim ml-1.5">({pct.toFixed(0)}%)</span>
        </span>
      </div>
      <Progress value={pct} glow={glow} />
    </div>
  );
}

function StatCell({
  icon: Icon,
  value,
  label,
  muted = false,
}: {
  icon: React.ComponentType<{ className?: string }>;
  value: number | undefined;
  label: string;
  muted?: boolean;
}) {
  return (
    <div className="text-center">
      <div
        className={cn(
          "flex items-center justify-center gap-1 mb-0.5",
          muted ? "text-text-dim" : "text-text",
        )}
      >
        <Icon className="h-3 w-3" />
        <span className="text-sm font-semibold tabular-nums">
          {value !== undefined ? value.toLocaleString() : "—"}
        </span>
      </div>
      <span className="text-[10px] uppercase tracking-wider text-text-dim">
        {label}
      </span>
    </div>
  );
}
