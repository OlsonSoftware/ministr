import {
  Database,
  HardDrive,
  FileText,
  Layers,
  Users,
  Clock,
  Hash,
  Sparkles,
  Activity,
  Folder,
} from "lucide-react";
import type { CorpusInfo, DaemonStatus } from "../lib/types";
import { statusBadge } from "../lib/status";
import { Badge } from "./ui/badge";
import { LabeledCard } from "./ui/labeled-card";
import { LabeledRow } from "./ui/labeled-row";
import { MetricTile } from "./ui/metric-tile";

interface ProjectDetailProps {
  corpus: CorpusInfo;
  status: DaemonStatus;
}

export function ProjectDetail({ corpus, status }: ProjectDetailProps) {
  const { variant: statusVariant, label: statusLabel } = statusBadge(corpus.status);
  return (
    <div className="space-y-4 ministr-fade-in">
      <header className="flex items-center justify-between gap-3">
        <div>
          <h2 className="text-base font-semibold text-text">Detail</h2>
          <p className="text-xs text-text-dim mt-0.5">
            Live metrics for this corpus.
          </p>
        </div>
        <Badge variant={statusVariant} dot>{statusLabel}</Badge>
      </header>

      <LabeledCard title="Index overview">
        <div className="grid grid-cols-2 gap-2.5">
          <MetricTile
            icon={FileText}
            label="Documents"
            value={corpus.files_indexed.toLocaleString()}
          />
          <MetricTile
            icon={Layers}
            label="Sections"
            value={corpus.sections_count.toLocaleString()}
          />
          <MetricTile
            icon={Database}
            label="Vectors"
            value={corpus.embeddings_count.toLocaleString()}
          />
          <MetricTile
            icon={Hash}
            label="Symbols"
            value={(corpus.symbols_count ?? 0).toLocaleString()}
          />
        </div>
      </LabeledCard>

      <LabeledCard title="Sessions">
        {corpus.active_sessions > 0 ? (
          <div className="flex items-center gap-3">
            <div className="grid h-9 w-9 place-items-center rounded-lg bg-[var(--color-accent-soft)] text-accent">
              <Users className="h-4 w-4" />
            </div>
            <div className="flex-1">
              <p className="text-sm font-semibold text-accent flex items-center gap-1.5">
                <span className="h-1.5 w-1.5 rounded-full bg-accent" />
                {corpus.active_sessions} active
              </p>
              <p className="text-xs text-text-dim">
                {corpus.active_sessions === 1
                  ? "MCP agent connection"
                  : "MCP agent connections"}
              </p>
            </div>
          </div>
        ) : (
          <div className="flex items-center gap-3">
            <div className="grid h-9 w-9 place-items-center rounded-lg bg-surface-overlay text-text-dim">
              <Users className="h-4 w-4" />
            </div>
            <div className="flex-1">
              <p className="text-sm font-medium text-text">No active sessions</p>
              <p className="text-xs text-text-dim">
                Connect an MCP client to start querying.
              </p>
            </div>
          </div>
        )}
      </LabeledCard>

      <LabeledCard title="Corpus ID" mono>
        <div className="font-mono text-[11px] leading-relaxed text-text-muted bg-surface-sunken border border-border/60 rounded-md px-3 py-2 break-all select-all">
          {corpus.id}
        </div>
      </LabeledCard>

      <LabeledCard title="Embedding model" icon={Sparkles}>
        <LabeledRow label="Model" value={status.model} mono />
        <LabeledRow
          label="Dimension"
          value={<Badge variant="muted" className="font-mono">{status.model_dimension}d</Badge>}
        />
      </LabeledCard>

      <LabeledCard title="Source paths" icon={Folder}>
        <ul className="space-y-1">
          {corpus.paths.map((path) => (
            <li
              key={path}
              className="flex items-center gap-2 text-[11px] font-mono text-text-dim"
            >
              <HardDrive className="h-3 w-3 shrink-0" />
              <span className="truncate" title={path}>
                {path}
              </span>
            </li>
          ))}
        </ul>
      </LabeledCard>

      <LabeledCard title="Daemon" icon={Activity}>
        <LabeledRow label="Version" value={`v${status.version}`} mono />
        <LabeledRow
          label="Uptime"
          mono
          value={
            <span className="inline-flex items-center gap-1">
              <Clock className="h-3 w-3 text-text-dim" />
              {formatUptime(status.uptime_secs)}
            </span>
          }
        />
        <LabeledRow label="Memory" value={`${status.memory_mb.toFixed(0)} MB RSS`} mono />
        <LabeledRow
          label="Corpora"
          value={status.corpora.length.toString()}
          mono
        />
      </LabeledCard>
    </div>
  );
}

function formatUptime(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}
