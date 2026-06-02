/**
 * IndexingPanel — the one component that renders live indexing state.
 *
 * Driven entirely by a {@link CorpusViewModel} (the consolidated source of
 * truth from `lib/corpusFleet`), so every surface that shows "what's happening
 * to this corpus right now" looks identical and stays in sync. Replaces the
 * per-surface ad-hoc "X / Y FILES + bar" lines with a granular, phase-aware
 * readout: a hero bar for the metric that's actually moving plus a
 * files/sections/vectors breakdown, live rate, ETA and current file.
 */
import { Box, FileText, Layers } from "lucide-react";

import { formatEta } from "../lib/format";
import {
  type CorpusViewModel,
  type IndexPhase,
  phaseLabel,
} from "../lib/corpusFleet";
import { cn } from "../lib/utils";
import type { BadgeVariant, Tone } from "../lib/status";
import { Badge } from "./ui/badge";
import { Progress } from "./ui/progress";

const PHASE_TONE: Record<IndexPhase, Tone> = {
  idle: "muted",
  queued: "muted",
  discovering: "accent",
  parsing: "accent",
  embedding: "warning",
  finalizing: "success",
  ready: "success",
  error: "danger",
};

const PHASE_BADGE: Record<IndexPhase, BadgeVariant> = {
  idle: "muted",
  queued: "muted",
  discovering: "default",
  parsing: "default",
  embedding: "warning",
  finalizing: "success",
  ready: "success",
  error: "danger",
};

const fmt = (n: number) => n.toLocaleString();

function Metric({
  icon: Icon,
  label,
  done,
  total,
  pct,
  tone,
}: {
  icon: typeof FileText;
  label: string;
  done: number;
  total?: number;
  pct?: number;
  tone: Tone;
}) {
  return (
    <div className="min-w-0">
      <div className="flex items-center gap-1.5 text-mono-mini font-mono uppercase tracking-[0.08em] text-text-dim">
        <Icon className="h-3 w-3 shrink-0" strokeWidth={2.5} />
        <span className="truncate">{label}</span>
      </div>
      <div className="mt-0.5 font-mono text-sm tabular-nums text-text leading-none">
        {fmt(done)}
        {total !== undefined && total > 0 && (
          <span className="text-text-dim">{` / ${fmt(total)}`}</span>
        )}
      </div>
      {total !== undefined && total > 0 && (
        <Progress className="mt-1.5 h-1" tone={tone} value={pct ?? 0} />
      )}
    </div>
  );
}

interface IndexingPanelProps {
  vm: CorpusViewModel;
  /** Compact spacing for the list card; default is the roomier detail view. */
  compact?: boolean;
  className?: string;
}

/** Granular, phase-aware live indexing readout. */
export function IndexingPanel({ vm, compact, className }: IndexingPanelProps) {
  const tone = PHASE_TONE[vm.phase];
  const { primary } = vm;

  return (
    <div className={cn(compact ? "space-y-2.5" : "space-y-3", className)}>
      {/* Phase + ETA */}
      <div className="flex items-center justify-between gap-2">
        <Badge variant={PHASE_BADGE[vm.phase]} dot>
          {phaseLabel(vm.phase)}
        </Badge>
        <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim tabular-nums">
          {vm.rate != null && vm.rate > 0 && (
            <span className="text-text-muted">
              {vm.rate >= 10 ? Math.round(vm.rate) : vm.rate.toFixed(1)}/s
            </span>
          )}
          {vm.rate != null && vm.rate > 0 && vm.etaSecs != null && (
            <span className="mx-1.5 text-border">·</span>
          )}
          {vm.etaSecs != null ? formatEta(vm.etaSecs) : "ETA …"}
        </span>
      </div>

      {/* Hero bar — the metric that's actually advancing */}
      <div>
        <div className="flex items-baseline justify-between gap-2 mb-1.5">
          <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-muted truncate">
            {primary.label}
            <span className="text-text-dim">
              {" · "}
              {fmt(primary.done)} / {fmt(primary.total)} {primary.unit}
            </span>
          </span>
          <span className="font-mono text-mono-mini tabular-nums text-text-muted shrink-0">
            {Math.round(primary.pct)}%
          </span>
        </div>
        <Progress tone={tone} value={primary.pct} glow className="h-2" />
      </div>

      {/* Granular breakdown — files / sections / vectors */}
      <div className="grid grid-cols-3 gap-3">
        <Metric
          icon={FileText}
          label="files"
          done={vm.files.done}
          total={vm.files.total}
          pct={vm.files.pct}
          tone="accent"
        />
        <Metric icon={Layers} label="sections" done={vm.sections} tone="muted" />
        <Metric
          icon={Box}
          label="vectors"
          done={vm.vectors.done}
          total={vm.vectors.total}
          pct={vm.vectors.pct}
          tone="warning"
        />
      </div>

      {/* Current file */}
      {vm.currentFile && (
        <p className="font-mono text-mono-mini text-text-dim truncate">
          {vm.currentFile}
        </p>
      )}
    </div>
  );
}
