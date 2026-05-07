import { useRef, useState } from "react";
import {
  Activity,
  Box,
  ChevronDown,
  ChevronRight,
  Clock,
  Code2,
  FileText,
  GitBranch,
  Layers,
  Network,
  TreePine,
  Users,
} from "lucide-react";
import type { CorpusInfo, DaemonStatus } from "../lib/types";
import { cn } from "../lib/utils";
import { Zone } from "./ui/zone";
import { CorpusTreemap } from "./CorpusTreemap";
import type { ExploreMode } from "./ExploreView";

interface ProjectDetailProps {
  corpus: CorpusInfo;
  status: DaemonStatus;
  /** Optional jump callback — if provided, ACTIONS shows quick-jumps.
   *  Pass `("explore", "symbols")` to land in Explore on the Symbols
   *  pivot. Structure is now an inline collapsible zone here, not a
   *  separate route. */
  onNavigate?: (target: "explore", exploreMode?: ExploreMode) => void;
}

/**
 * Three-zone detail pane: STATS · METADATA · ACTIONS.
 * Replaces the previous 6-section stack of separate cards.
 *
 * Re-index and remove actions live in ProjectList (the typed-confirm
 * modal) — duplicating them here previously rendered permanently
 * disabled because App.tsx never wired the callbacks. Keep ACTIONS
 * focused on the navigate-into-tab quick-jumps.
 */
export function ProjectDetail({
  corpus,
  status,
  onNavigate,
}: ProjectDetailProps) {
  const indexing = corpus.status.state === "indexing";
  const error = corpus.status.state === "error" ? corpus.status.message : null;
  const [structureExpanded, setStructureExpanded] = useState(false);
  const structureRef = useRef<HTMLDivElement>(null);

  function jumpToStructure() {
    setStructureExpanded(true);
    // Scroll into view on the next frame so the expansion has a chance
    // to layout before we measure.
    requestAnimationFrame(() => {
      structureRef.current?.scrollIntoView({
        behavior: "smooth",
        block: "start",
      });
    });
  }

  return (
    <div className="space-y-4">
      {/* STATS */}
      <Zone title="STATS">
        <div className="grid grid-cols-2 gap-0">
          <Stat icon={FileText} label="FILES" value={corpus.files_indexed} />
          <Stat icon={Layers} label="SECTIONS" value={corpus.sections_count} />
          <Stat icon={Code2} label="SYMBOLS" value={corpus.symbols_count ?? 0} />
          <Stat
            icon={Box}
            label="VECTORS"
            value={corpus.embeddings_count}
          />
        </div>

        <div className="border-t-2 border-border px-3 py-2 flex items-center justify-between">
          <span className="font-sans text-xs tracking-[0.05em] text-text-dim">
            Sessions
          </span>
          <span
            className={cn(
              "font-mono text-sm font-bold tabular-nums",
              corpus.active_sessions > 0 ? "text-accent" : "text-text-muted",
            )}
          >
            {corpus.active_sessions}
          </span>
        </div>

        {indexing && (
          <div className="border-t-2 border-border px-3 py-2 flex items-center gap-2 text-warning">
            <span className="h-1.5 w-1.5 bg-warning ministr-blink shrink-0" />
            <span className="font-sans text-xs font-bold tracking-[0.05em]">
              Indexing in progress
            </span>
          </div>
        )}
        {error && (
          <div className="border-t-2 border-danger px-3 py-2 text-danger">
            <p className="font-mono text-xs font-bold tracking-[0.05em] mb-1">
              ERROR
            </p>
            <p className="font-mono text-[0.6875rem] leading-relaxed break-words">
              {error}
            </p>
          </div>
        )}
      </Zone>

      {/* METADATA */}
      <Zone title="METADATA" subtitle="READ-ONLY">
        <Row label="CORPUS ID" value={corpus.id} mono />
        <Row label="MODEL" value={status.model} mono />
        <Row label="DIM" value={`${status.model_dimension}d`} mono />
        <Row
          label="MEMORY"
          value={`${status.memory_mb.toFixed(0)} MB RSS`}
          mono
        />
        <Row
          label="DAEMON"
          value={
            <span className="inline-flex items-center gap-1">
              v{status.version}
              <span className="text-text-dim">·</span>
              <Clock className="h-3 w-3 text-text-dim" strokeWidth={2.5} />
              <span className="tabular-nums">
                {formatUptime(status.uptime_secs)}
              </span>
            </span>
          }
          mono
        />
        <div className="border-b-2 border-border last:border-b-0 px-3 py-2">
          <div className="font-sans text-xs tracking-[0.05em] text-text-dim mb-1">
            Source paths
          </div>
          <ul className="space-y-0.5">
            {corpus.paths.map((path) => (
              <li
                key={path}
                className="flex items-start gap-2 font-mono text-[0.6875rem] text-text break-all"
                title={path}
              >
                <span className="text-text-dim shrink-0 mt-0.5">·</span>
                <span>{path}</span>
              </li>
            ))}
          </ul>
        </div>
      </Zone>

      {/* ACTIONS */}
      <Zone title="ACTIONS">
        <div className="grid grid-cols-3 gap-0 p-3">
          <ActionButton
            icon={GitBranch}
            label="SYMBOLS"
            onClick={() => onNavigate?.("explore", "symbols")}
          />
          <ActionButton
            icon={Network}
            label="BRIDGE"
            onClick={() => onNavigate?.("explore", "bridges")}
          />
          <ActionButton
            icon={TreePine}
            label="STRUCTURE"
            onClick={jumpToStructure}
          />
        </div>
      </Zone>

      {/* STRUCTURE — inline collapsible treemap. Replaces the prior
          standalone tab; lives where the user already is when poking at
          a corpus. Collapsed by default to keep the project pane fast. */}
      <div ref={structureRef}>
        <Zone
          title="STRUCTURE"
          subtitle={structureExpanded ? "TREEMAP" : "COLLAPSED"}
        >
          <button
            onClick={() => setStructureExpanded((e) => !e)}
            className="flex w-full items-center gap-2 border-b-2 border-border px-3 py-2 cursor-pointer hover:bg-surface-overlay transition-none text-left"
          >
            {structureExpanded ? (
              <ChevronDown className="h-3.5 w-3.5 text-text-dim" strokeWidth={2.5} />
            ) : (
              <ChevronRight className="h-3.5 w-3.5 text-text-dim" strokeWidth={2.5} />
            )}
            <span className="font-sans text-xs tracking-[0.05em] text-text-muted">
              {structureExpanded
                ? "Hide treemap"
                : "Show file-size treemap"}
            </span>
          </button>
          {structureExpanded && (
            <div className="p-3 max-h-[600px] overflow-y-auto">
              <CorpusTreemap
                status={status}
                activeCorpusId={corpus.id}
                setActiveCorpusId={() => {
                  /* no-op — corpus selection is owned by the parent */
                }}
                onNavigate={onNavigate}
              />
            </div>
          )}
        </Zone>
      </div>
    </div>
  );
}

function Stat({
  icon: Icon,
  label,
  value,
}: {
  icon: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  label: string;
  value: number;
}) {
  return (
    <div className="border-r border-b border-border-soft [&:nth-child(2n)]:border-r-0 [&:nth-last-child(-n+2)]:border-b-0 px-3 py-2.5 flex items-center gap-3 min-w-0">
      <div className="grid h-7 w-7 place-items-center border border-border-soft bg-surface-overlay text-text shrink-0">
        <Icon className="h-3.5 w-3.5" strokeWidth={2.5} />
      </div>
      <div className="min-w-0 flex-1">
        <p className="font-mono text-xs font-bold tracking-[0.05em] text-text-dim">
          {label}
        </p>
        <p className="font-mono text-base font-bold tabular-nums text-text">
          {value.toLocaleString()}
        </p>
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
    <div className="flex items-center justify-between gap-3 border-b-2 border-border last:border-b-0 px-3 py-1.5">
      <span className="font-mono text-xs tracking-[0.05em] text-text-dim shrink-0">
        {label}
      </span>
      <span
        className={cn(
          "text-text text-right truncate",
          mono ? "font-mono text-[0.6875rem] tabular-nums" : "text-xs",
        )}
        title={typeof value === "string" ? value : undefined}
      >
        {value}
      </span>
    </div>
  );
}

function ActionButton({
  icon: Icon,
  label,
  onClick,
}: {
  icon: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  label: string;
  onClick?: () => void;
}) {
  return (
    <button
      onClick={onClick}
      disabled={!onClick}
      className={cn(
        "border-2 border-border -ml-[2px] first:ml-0 px-2 py-3 flex flex-col items-center justify-center gap-1.5 cursor-pointer transition-none",
        "bg-surface text-text hover:bg-surface-overlay hover:text-text",
        !onClick && "opacity-40 cursor-not-allowed",
      )}
    >
      <Icon className="h-4 w-4" strokeWidth={2.5} />
      <span className="font-mono text-xs font-bold tracking-[0.05em]">
        → {label}
      </span>
    </button>
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

// Re-export `Activity` `Users` to keep the prior import surface stable.
// (No direct usage anymore — kept here for one-line clarity.)
void Activity;
void Users;
