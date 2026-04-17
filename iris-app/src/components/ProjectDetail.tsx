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
import type { CorpusInfo, DaemonStatus, IndexingStatus } from "../lib/types";
import { Card } from "./ui/card";
import { Badge } from "./ui/badge";
import { cn } from "../lib/utils";

interface ProjectDetailProps {
  corpus: CorpusInfo;
  status: DaemonStatus;
}

function statusBadge(status: IndexingStatus) {
  switch (status.state) {
    case "idle":
      return <Badge variant="success" dot>Ready</Badge>;
    case "indexing":
      return <Badge variant="warning" dot>Indexing</Badge>;
    case "error":
      return <Badge variant="danger" dot>Error</Badge>;
  }
}

export function ProjectDetail({ corpus, status }: ProjectDetailProps) {
  return (
    <div className="space-y-4 iris-fade-in">
      <header className="flex items-center justify-between gap-3">
        <div>
          <h2 className="text-base font-semibold text-text">Detail</h2>
          <p className="text-xs text-text-dim mt-0.5">
            Live metrics for this corpus.
          </p>
        </div>
        {statusBadge(corpus.status)}
      </header>

      <Section title="Index overview">
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
      </Section>

      <Section title="Sessions">
        {corpus.active_sessions > 0 ? (
          <div className="flex items-center gap-3">
            <div className="grid h-9 w-9 place-items-center rounded-lg bg-[var(--color-accent-soft)] text-accent">
              <Users className="h-4 w-4" />
            </div>
            <div className="flex-1">
              <p className="text-sm font-semibold text-accent flex items-center gap-1.5">
                <span className="iris-pulse h-1.5 w-1.5 rounded-full bg-accent" />
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
      </Section>

      <Section title="Corpus ID" mono>
        <div className="font-mono text-[11px] leading-relaxed text-text-muted bg-surface-sunken border border-border/60 rounded-md px-3 py-2 break-all select-all">
          {corpus.id}
        </div>
      </Section>

      <Section title="Embedding model" icon={Sparkles}>
        <Row label="Model" value={status.model} mono />
        <Row
          label="Dimension"
          value={<Badge variant="muted" className="font-mono">{status.model_dimension}d</Badge>}
        />
      </Section>

      <Section title="Source paths" icon={Folder}>
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
      </Section>

      <Section title="Daemon" icon={Activity}>
        <Row label="Version" value={`v${status.version}`} mono />
        <Row
          label="Uptime"
          mono
          value={
            <span className="inline-flex items-center gap-1">
              <Clock className="h-3 w-3 text-text-dim" />
              {formatUptime(status.uptime_secs)}
            </span>
          }
        />
        <Row label="Memory" value={`${status.memory_mb.toFixed(0)} MB RSS`} mono />
        <Row
          label="Corpora"
          value={status.corpora.length.toString()}
          mono
        />
      </Section>
    </div>
  );
}

function Section({
  title,
  children,
  mono = false,
  icon: Icon,
}: {
  title: string;
  children: React.ReactNode;
  mono?: boolean;
  icon?: React.ComponentType<{ className?: string }>;
}) {
  return (
    <Card hover="lift" className={cn(mono && "p-3")}>
      <div className="flex items-center gap-1.5 mb-2.5">
        {Icon && <Icon className="h-3.5 w-3.5 text-text-dim" />}
        <h3 className="text-[11px] font-semibold uppercase tracking-wider text-text-dim">
          {title}
        </h3>
      </div>
      <div className="space-y-1.5">{children}</div>
    </Card>
  );
}

function MetricTile({
  icon: Icon,
  label,
  value,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  value: string;
}) {
  return (
    <div className="flex items-center gap-2.5 rounded-lg border border-border/50 bg-surface/40 px-2.5 py-2">
      <div className="grid h-7 w-7 place-items-center rounded-md bg-surface-overlay text-text-muted">
        <Icon className="h-3.5 w-3.5" />
      </div>
      <div className="min-w-0 flex-1">
        <p className="text-[10px] font-medium uppercase tracking-wider text-text-dim">
          {label}
        </p>
        <p className="text-sm font-semibold tabular-nums text-text truncate">
          {value}
        </p>
      </div>
    </div>
  );
}

function Row({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: React.ReactNode;
  mono?: boolean;
}) {
  return (
    <div className="flex items-center justify-between text-xs">
      <span className="text-text-muted">{label}</span>
      <span className={cn("text-text", mono && "font-mono tabular-nums")}>
        {value}
      </span>
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
