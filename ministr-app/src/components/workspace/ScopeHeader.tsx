import { Layers } from "lucide-react";
import { useWorkspace } from "./WorkspaceContext";
import { corpusLabel, corpusRoot } from "../../lib/corpus";
import { corpusTone, isCorpusLive } from "../../lib/status";
import { StatusDot } from "../ui/status-dot";

/**
 * The scope header — renders the spine object's identity + key stats.
 *
 * Every facet sits beneath this, so switching facets visibly keeps the SAME
 * object in view: that is the "one context" integration test made visible.
 * It reads only `useWorkspace()`, never a per-facet selection, and the real
 * facets reuse it once they mount.
 */
export function ScopeHeader() {
  const { isFleet, activeProject, corpora } = useWorkspace();

  if (isFleet) {
    const projects = corpora.length;
    const files = corpora.reduce((n, c) => n + c.files_indexed, 0);
    const symbols = corpora.reduce((n, c) => n + c.symbols_count, 0);
    const live = corpora.filter((c) => isCorpusLive(c)).length;
    return (
      <Frame>
        <div className="flex items-center gap-3 min-w-0">
          <span
            className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-border bg-surface-overlay text-text-muted"
            aria-hidden
          >
            <Layers className="h-[18px] w-[18px]" strokeWidth={2} />
          </span>
          <div className="min-w-0">
            <div className="font-sans text-sm font-semibold text-text">Fleet</div>
            <div className="font-mono text-mono-mini text-text-dim truncate">
              all projects · zoomed out
            </div>
          </div>
        </div>
        <StatCluster>
          <Stat label="projects" value={projects} />
          <Stat label="live" value={live} />
          <Stat label="files" value={files} />
          <Stat label="symbols" value={symbols} />
        </StatCluster>
      </Frame>
    );
  }

  if (!activeProject) {
    return (
      <Frame>
        <div className="font-sans text-sm text-text-dim">No project selected</div>
      </Frame>
    );
  }

  const c = activeProject;
  const root = corpusRoot(c.paths);
  return (
    <Frame>
      <div className="flex items-center gap-3 min-w-0">
        <span
          className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-border bg-surface-overlay"
          aria-hidden
        >
          <StatusDot
            tone={corpusTone(c)}
            pulse={isCorpusLive(c) ? "live" : "off"}
          />
        </span>
        <div className="min-w-0">
          <div className="font-mono text-sm font-semibold text-text truncate">
            {corpusLabel(c)}
          </div>
          {root && (
            <div className="font-mono text-mono-mini text-text-dim truncate">
              {root}
            </div>
          )}
        </div>
      </div>
      <StatCluster>
        <Stat label="files" value={c.files_indexed} />
        <Stat label="sections" value={c.sections_count} />
        <Stat label="symbols" value={c.symbols_count} />
        {c.model && <Stat label="model" value={c.model} mono />}
      </StatCluster>
    </Frame>
  );
}

function Frame({ children }: { children: React.ReactNode }) {
  return (
    <header className="flex flex-wrap items-center justify-between gap-x-6 gap-y-3 border-b border-border bg-surface/40 px-5 py-3.5">
      {children}
    </header>
  );
}

function StatCluster({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex items-center gap-5 font-mono text-mono-mini text-text-dim">
      {children}
    </div>
  );
}

function Stat({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: number | string;
  mono?: boolean;
}) {
  const display =
    typeof value === "number" ? value.toLocaleString() : value;
  return (
    <span className="flex items-baseline gap-1.5">
      <span
        className={
          mono
            ? "text-text tabular-nums max-w-[160px] truncate"
            : "text-text tabular-nums font-semibold"
        }
      >
        {display}
      </span>
      <span className="uppercase tracking-[0.08em] text-[0.92em]">{label}</span>
    </span>
  );
}
