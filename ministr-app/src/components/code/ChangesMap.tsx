/**
 * ChangesMap — the Explore "Changes" lens: a branch DIFF as a first-class
 * object (FL7's review / targeted-PR-context station).
 *
 * A revision range (e.g. `main..HEAD`) becomes a queryable object with three
 * facets: WHAT changed (the symbols the diff touched), WHO owns it (per-symbol
 * git blame), and WHAT IT CAN BREAK (the union blast radius over the reference
 * graph). Drives the `diff_impact` command — the GUI mirror of FL7's
 * `ministr_impact` range op. Built fresh from the v4 tokens/atoms; consistent
 * with the Code | Bridges | Unused | Quality | Diagnostics lens vocabulary.
 *
 * Pure `ChangesMap` renders from props (Storybook); `ChangesMapConnector` wires
 * the `diff_impact` invoke, the range input, and the shared inspector.
 */
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  CornerDownLeft,
  FileCode2,
  GitBranch,
  GitCompareArrows,
  ShieldAlert,
  TriangleAlert,
  Users,
  Waypoints,
} from "lucide-react";

import type { ChangedSymbol, DiffImpact, ImpactedSymbol, SymbolInfo } from "../../lib/types";
import { cn } from "../../lib/utils";
import { useEntityPanel } from "../../hooks/useEntityPanel";
import { useArrowKeyListNav } from "../../hooks/useArrowKeyListNav";
import { LensLoading, LensEmpty } from "../ui/lens-frame";

function baseName(path: string): string {
  return path.replace(/\\/g, "/").split("/").pop() ?? path;
}

function fileTail(path: string): string {
  return path.replace(/\\/g, "/").split("/").slice(-2).join("/");
}

function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  if (parts.length === 1) return parts[0]!.slice(0, 2).toUpperCase();
  return (parts[0]![0]! + parts[parts.length - 1]![0]!).toUpperCase();
}

/** Deterministic hue from a name so each author keeps a stable accent dot. */
function authorHue(name: string): number {
  let h = 0;
  for (let i = 0; i < name.length; i += 1) h = (h * 31 + name.charCodeAt(i)) % 360;
  return h;
}

const RISK_META: Record<DiffImpact["risk"], { label: string; chip: string; icon: typeof TriangleAlert }> = {
  low: {
    label: "Low risk",
    chip: "border-success/40 bg-success/10 text-success",
    icon: ShieldAlert,
  },
  medium: {
    label: "Medium risk",
    chip: "border-warning/40 bg-warning/10 text-warning",
    icon: TriangleAlert,
  },
  high: {
    label: "High risk",
    chip: "border-danger/40 bg-danger/10 text-danger",
    icon: TriangleAlert,
  },
};

export interface ChangesMapProps {
  /** The resolved diff, or null before a range has been run. */
  data: DiffImpact | null;
  loading?: boolean;
  /** A range/git error message to surface (e.g. not a git checkout). */
  error?: string | null;
  /** Whether a repo path is available (false → not a git checkout). */
  hasRepo?: boolean;
  /** The current range text in the input. */
  range: string;
  onRangeChange: (range: string) => void;
  /** Run the diff for the current range. */
  onRun: () => void;
  /** Inspect a symbol in the shared EntityPanel. */
  onInspect: (symbolId: string, name: string, kind: string, file: string) => void;
  /** Jump to a symbol's location in the code lens. */
  onOpenFile: (path: string, line: number) => void;
}

export function ChangesMap({
  data,
  loading = false,
  error = null,
  hasRepo = true,
  range,
  onRangeChange,
  onRun,
  onInspect,
  onOpenFile,
}: ChangesMapProps) {
  // Changed symbols grouped by file (most-changed file first).
  const groups = useMemo(() => {
    if (!data) return [];
    const byFile = new Map<string, ChangedSymbol[]>();
    for (const s of data.changed_symbols) {
      const arr = byFile.get(s.file);
      if (arr) arr.push(s);
      else byFile.set(s.file, [s]);
    }
    for (const arr of byFile.values()) arr.sort((a, b) => a.line - b.line);
    return [...byFile.entries()].sort((a, b) => b[1].length - a[1].length);
  }, [data]);

  const listRef = useArrowKeyListNav<HTMLDivElement>();

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* ── Range bar — the diff object's identity + a glance summary. ──── */}
      <header className="shrink-0 border-b border-border-soft bg-surface px-4 py-3 space-y-2.5">
        <div className="flex items-center gap-2 text-accent">
          <GitCompareArrows className="h-4 w-4" strokeWidth={2} />
          <span className="font-mono text-xs font-bold uppercase tracking-[0.08em]">Changes</span>
          <span className="font-mono text-mono-micro text-text-dim">
            diff-aware blast radius · what a branch touched & what it can break
          </span>
        </div>

        <form
          className="flex items-center gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (hasRepo) onRun();
          }}
        >
          <div className="flex min-w-0 flex-1 items-center gap-2 rounded-md border border-border-soft bg-surface-sunken px-2.5 py-1.5 focus-within:border-accent">
            <GitBranch className="h-3.5 w-3.5 shrink-0 text-text-dim" strokeWidth={2} />
            <input
              value={range}
              onChange={(e) => onRangeChange(e.target.value)}
              spellCheck={false}
              placeholder="main..HEAD"
              aria-label="Git revision range"
              disabled={!hasRepo}
              className="min-w-0 flex-1 bg-transparent font-mono text-xs text-text placeholder:text-text-dim focus:outline-none disabled:opacity-50"
            />
            <kbd className="hidden shrink-0 items-center gap-0.5 rounded border border-border-soft px-1 font-mono text-mono-micro text-text-dim sm:inline-flex">
              <CornerDownLeft className="h-2.5 w-2.5" strokeWidth={2} />
            </kbd>
          </div>
          <button
            type="submit"
            disabled={!hasRepo || loading}
            className="shrink-0 rounded-md border border-accent bg-accent/10 px-2.5 py-1.5 font-mono text-mono-mini font-semibold uppercase tracking-[0.06em] text-accent hover:bg-accent/20 disabled:opacity-50 cursor-pointer transition-colors duration-150"
          >
            Diff
          </button>
        </form>

        {data && data.changed_symbols.length > 0 && <GlanceRow data={data} />}
      </header>

      {/* ── Body. ──────────────────────────────────────────────────────── */}
      {!hasRepo ? (
        <LensEmpty
          icon={GitBranch}
          title="Not a git checkout"
          hint="This corpus has no git work tree, so there's no branch diff to review. Open a project that's a git repository to see what a range changed and what it can break."
        />
      ) : error ? (
        <LensEmpty
          icon={TriangleAlert}
          title="Couldn't resolve that range"
          hint={error}
        />
      ) : loading ? (
        <LensLoading label="Resolving the diff" />
      ) : !data ? (
        <LensEmpty
          icon={GitCompareArrows}
          accent
          title="Review a branch"
          hint="Enter a range like main..HEAD or HEAD~3 and press Diff. ministr resolves which indexed symbols the change touched, who wrote them, and the union blast radius — what the branch can break."
        />
      ) : data.changed_symbols.length === 0 ? (
        <LensEmpty
          icon={GitCompareArrows}
          title="Nothing indexed changed"
          hint={`No indexed symbols were touched by ${data.range}. The range may be empty, touch only un-indexed files (config, docs), or fall outside symbol bodies.`}
        />
      ) : (
        <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto">
          {/* WHAT changed — symbols grouped by file, with authorship. */}
          {groups.map(([file, syms]) => (
            <section key={file} className="border-b border-border-soft last:border-b-0">
              <header className="sticky top-0 z-10 flex items-center gap-2 border-b border-border-soft bg-surface-overlay/95 px-4 py-1.5 backdrop-blur">
                <FileCode2 className="h-3.5 w-3.5 text-text-dim" strokeWidth={2} />
                <button
                  type="button"
                  onClick={() => onOpenFile(file, syms[0]?.line ?? 1)}
                  title={`Open ${file}`}
                  className="truncate font-mono text-mono-micro font-bold uppercase tracking-[0.06em] text-text hover:text-accent cursor-pointer transition-colors duration-150"
                >
                  {baseName(file)}
                </button>
                <span className="truncate font-mono text-mono-micro text-text-dim">{fileTail(file)}</span>
                <span className="ml-auto shrink-0 font-mono text-mono-micro tabular-nums text-text-dim">
                  {syms.length} changed
                </span>
              </header>
              <div className="divide-y divide-border-soft/60">
                {syms.map((s) => (
                  <ChangedRow
                    key={s.symbol_id || `${s.file}:${s.line}`}
                    sym={s}
                    onInspect={() => onInspect(s.symbol_id, s.name, s.kind, s.file)}
                    onOpenFile={onOpenFile}
                  />
                ))}
              </div>
            </section>
          ))}

          {/* WHAT IT CAN BREAK — the union blast radius. */}
          <BlastRadius impacted={data.impacted} onInspect={onInspect} onOpenFile={onOpenFile} />
        </div>
      )}
    </div>
  );
}

function GlanceRow({ data }: { data: DiffImpact }) {
  const risk = RISK_META[data.risk];
  const RiskIcon = risk.icon;
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
      <span className="font-mono text-mono-mini text-text-dim">
        <span className="tabular-nums font-semibold text-text">{data.changed_symbols.length}</span>{" "}
        changed ·{" "}
        <span className="tabular-nums font-semibold text-text">{data.changed_files}</span> files
      </span>
      <span
        className={cn(
          "inline-flex items-center gap-1 rounded border px-1.5 py-0.5 font-mono text-mono-micro font-semibold uppercase tracking-[0.06em]",
          risk.chip,
        )}
        title="Aggregate blast-radius risk"
      >
        <RiskIcon className="h-3 w-3" strokeWidth={2.25} />
        {risk.label}
      </span>
      <span className="inline-flex items-center gap-1 font-mono text-mono-mini text-text-dim">
        <Waypoints className="h-3 w-3 text-accent" strokeWidth={2} />
        <span className="tabular-nums font-semibold text-text">{data.impacted_symbols}</span> impacted
        · <span className="tabular-nums">{data.impacted_files}</span> files ·{" "}
        <span className={cn("tabular-nums", data.impacted_tests > 0 && "text-success")}>
          {data.impacted_tests}
        </span>{" "}
        tests
      </span>
    </div>
  );
}

function ChangedRow({
  sym,
  onInspect,
  onOpenFile,
}: {
  sym: ChangedSymbol;
  onInspect: () => void;
  onOpenFile: (path: string, line: number) => void;
}) {
  return (
    <div
      role="button"
      tabIndex={0}
      data-roving-item
      onClick={onInspect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onInspect();
        }
      }}
      title={`Inspect ${sym.name}`}
      className="group flex items-center gap-2.5 px-4 py-2 cursor-pointer hover:bg-surface-overlay transition-colors duration-150 ease-out"
    >
      <span className="shrink-0 rounded border border-border-soft bg-surface px-1 font-mono text-mono-micro lowercase tracking-[0.04em] text-text-dim">
        {sym.kind || "sym"}
      </span>
      <span className="truncate font-mono text-xs font-semibold text-text">{sym.name}</span>
      <AuthorChips authors={sym.authors} />
      <span className="flex-1" />
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onOpenFile(sym.file, sym.line);
        }}
        title={`Open ${sym.file}:${sym.line}`}
        className="shrink-0 font-mono text-mono-micro tabular-nums text-text-dim hover:text-accent cursor-pointer transition-colors duration-150"
      >
        :{sym.line}
      </button>
    </div>
  );
}

function AuthorChips({ authors }: { authors: ChangedSymbol["authors"] }) {
  if (authors.length === 0) return null;
  const shown = authors.slice(0, 3);
  const extra = authors.length - shown.length;
  return (
    <span className="hidden shrink-0 items-center gap-1 md:inline-flex" title="Authors (git blame)">
      {shown.map((a) => (
        <span
          key={a.name}
          className="inline-flex items-center gap-1 rounded-full border border-border-soft bg-surface px-1 py-0.5"
          title={`${a.name} · ${a.lines} ${a.lines === 1 ? "line" : "lines"}`}
        >
          <span
            className="grid h-3.5 w-3.5 place-items-center rounded-full font-mono text-[8px] font-bold text-bg"
            style={{ backgroundColor: `hsl(${authorHue(a.name)} 55% 60%)` }}
          >
            {initials(a.name)}
          </span>
          <span className="max-w-[7rem] truncate font-mono text-mono-micro text-text-dim">
            {a.name}
          </span>
        </span>
      ))}
      {extra > 0 && (
        <span className="font-mono text-mono-micro text-text-dim" title={`${extra} more`}>
          +{extra}
        </span>
      )}
    </span>
  );
}

function BlastRadius({
  impacted,
  onInspect,
  onOpenFile,
}: {
  impacted: ImpactedSymbol[];
  onInspect: (symbolId: string, name: string, kind: string, file: string) => void;
  onOpenFile: (path: string, line: number) => void;
}) {
  if (impacted.length === 0) {
    return (
      <section className="px-4 py-6">
        <p className="flex items-center gap-2 font-mono text-mono-mini text-text-dim">
          <Waypoints className="h-3.5 w-3.5 text-success" strokeWidth={2} />
          Nothing else references the changed symbols — an isolated change.
        </p>
      </section>
    );
  }
  return (
    <section>
      <header className="sticky top-0 z-10 flex items-center gap-2 border-y border-border-soft bg-surface-overlay/95 px-4 py-1.5 backdrop-blur">
        <Waypoints className="h-3.5 w-3.5 text-accent" strokeWidth={2} />
        <span className="font-mono text-mono-micro font-bold uppercase tracking-[0.06em] text-text">
          Blast radius
        </span>
        <span className="font-mono text-mono-micro text-text-dim">
          what reaches the changed symbols (incoming)
        </span>
        <span className="ml-auto flex items-center gap-1 font-mono text-mono-micro text-text-dim">
          <Users className="h-3 w-3" strokeWidth={2} />
          <span className="tabular-nums font-semibold text-text">{impacted.length}</span>
        </span>
      </header>
      <div className="divide-y divide-border-soft/60">
        {impacted.map((c) => (
          <div
            key={c.symbol_id || `${c.file}:${c.line}`}
            role="button"
            tabIndex={0}
            data-roving-item
            onClick={() => onInspect(c.symbol_id, c.name, c.kind, c.file)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onInspect(c.symbol_id, c.name, c.kind, c.file);
              }
            }}
            title={`Inspect ${c.name}`}
            className="group flex items-center gap-2.5 px-4 py-1.5 cursor-pointer hover:bg-surface-overlay transition-colors duration-150"
          >
            <span
              className="shrink-0 rounded-full border border-border-soft px-1.5 font-mono text-mono-micro tabular-nums text-text-dim"
              title={`${c.depth} hop${c.depth === 1 ? "" : "s"} from a changed symbol`}
            >
              {c.depth}↑
            </span>
            <span className="truncate font-mono text-xs text-text">{c.name}</span>
            <span className="truncate font-mono text-mono-micro text-text-dim">{fileTail(c.file)}</span>
            <span className="flex-1" />
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                onOpenFile(c.file, c.line);
              }}
              title={`Open ${c.file}:${c.line}`}
              className="shrink-0 font-mono text-mono-micro tabular-nums text-text-dim hover:text-accent cursor-pointer transition-colors duration-150"
            >
              :{c.line}
            </button>
          </div>
        ))}
      </div>
    </section>
  );
}

// ───────────────────────────────────────────────────────────────────────────
// Connector — runs the diff_impact command + wires the shared inspector.

function toSymbolInfo(symbolId: string, name: string, kind: string, file: string): SymbolInfo {
  return {
    id: symbolId,
    name,
    kind,
    file_path: file,
    visibility: "",
    signature: name,
    doc_comment: null,
    module_path: "",
  };
}

export function ChangesMapConnector({
  corpusId,
  repoPath,
  onOpenFile,
}: {
  corpusId: string;
  repoPath: string | null;
  onOpenFile: (path: string, line: number) => void;
}) {
  const { openEntity } = useEntityPanel();
  const [range, setRange] = useState("main..HEAD");
  const [data, setData] = useState<DiffImpact | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function run(r: string) {
    if (!repoPath) return;
    setLoading(true);
    setError(null);
    // The daemon resolves the repo from the corpus's local root; `repoPath`
    // stays only as the local "is this a git checkout?" UX gate (`hasRepo`).
    invoke<DiffImpact>("diff_impact", { corpusId, range: r, maxDepth: 3 })
      .then((d) => {
        setData(d);
        setLoading(false);
      })
      .catch((e: unknown) => {
        setError(String(e));
        setData(null);
        setLoading(false);
      });
  }

  // Auto-run main..HEAD on first mount / corpus change for instant value.
  useEffect(() => {
    setData(null);
    setError(null);
    if (repoPath) run("main..HEAD");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [corpusId, repoPath]);

  return (
    <ChangesMap
      data={data}
      loading={loading}
      error={error}
      hasRepo={!!repoPath}
      range={range}
      onRangeChange={setRange}
      onRun={() => run(range)}
      onInspect={(symbolId, name, kind, file) => {
        if (symbolId) {
          openEntity({ kind: "symbol", corpusId, symbol: toSymbolInfo(symbolId, name, kind, file) });
        }
      }}
      onOpenFile={onOpenFile}
    />
  );
}
