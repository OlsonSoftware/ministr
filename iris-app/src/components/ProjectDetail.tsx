import {
  Database,
  Cpu,
  HardDrive,
  GitBranch,
  Code2,
  FileText,
  Layers,
} from "lucide-react";
import type { CorpusInfo, DaemonStatus } from "../lib/types";
import { Card } from "./ui/card";
import { Badge } from "./ui/badge";

interface ProjectDetailProps {
  corpus: CorpusInfo;
  status: DaemonStatus;
}

export function ProjectDetail({ corpus, status }: ProjectDetailProps) {
  return (
    <div className="space-y-3">
      <h2 className="text-sm font-medium text-text-muted uppercase tracking-wider">
        Detail
      </h2>

      <Card>
        <h3 className="font-medium text-sm mb-3">Index Overview</h3>
        <div className="grid grid-cols-2 gap-3">
          <Stat icon={FileText} label="Documents" value={corpus.files_indexed} />
          <Stat icon={Layers} label="Sections" value={corpus.sections_count} />
          <Stat icon={Database} label="Vectors" value={corpus.embeddings_count} />
          <Stat icon={Code2} label="Status" value={corpus.status.state} />
        </div>
      </Card>

      <Card>
        <h3 className="font-medium text-sm mb-3">Embedding Model</h3>
        <div className="space-y-2 text-xs">
          <div className="flex justify-between">
            <span className="text-text-muted">Model</span>
            <span className="font-mono">{status.model}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-text-muted">Dimension</span>
            <Badge variant="muted">{status.model_dimension}d</Badge>
          </div>
          <div className="flex justify-between">
            <span className="text-text-muted">Memory</span>
            <span>{status.memory_mb.toFixed(0)} MB</span>
          </div>
        </div>
      </Card>

      <Card>
        <h3 className="font-medium text-sm mb-3">Source Paths</h3>
        <div className="space-y-1">
          {corpus.paths.map((path) => (
            <div
              key={path}
              className="flex items-center gap-2 text-xs text-text-dim font-mono"
            >
              <HardDrive className="h-3 w-3 shrink-0" />
              <span className="truncate">{path}</span>
            </div>
          ))}
        </div>
      </Card>

      <Card>
        <h3 className="font-medium text-sm mb-3">Daemon</h3>
        <div className="space-y-2 text-xs">
          <div className="flex justify-between">
            <span className="text-text-muted">Version</span>
            <span>v{status.version}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-text-muted">Uptime</span>
            <span>{formatUptime(status.uptime_secs)}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-text-muted">Corpora</span>
            <span>{status.corpora.length}</span>
          </div>
        </div>
      </Card>
    </div>
  );
}

function Stat({
  icon: Icon,
  label,
  value,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  value: string | number;
}) {
  return (
    <div className="flex items-center gap-2">
      <div className="rounded-md bg-surface-overlay p-1.5">
        <Icon className="h-3.5 w-3.5 text-text-muted" />
      </div>
      <div>
        <p className="text-xs text-text-dim">{label}</p>
        <p className="text-sm font-medium">{String(value)}</p>
      </div>
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
