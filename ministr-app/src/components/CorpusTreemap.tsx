import { useState, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TreePine } from "lucide-react";
import { cn } from "../lib/utils";
import { corpusRelative } from "../lib/path";
import { useEntityPanel } from "../hooks/useEntityPanel";
import type { DaemonStatus, FileInfo } from "../lib/types";
import type { ExploreMode } from "./ExploreView";
import { H1 } from "./ui/heading";

interface Props {
  status: DaemonStatus;
  activeCorpusId: string | null;
  setActiveCorpusId: (id: string | null) => void;
  /** Optional jump callback — clicking a cell navigates to the
   *  Explore tab on the given pivot mode. Currently unused inside
   *  the treemap body; reserved for future drill-in actions. */
  onNavigate?: (target: "explore", exploreMode?: ExploreMode) => void;
}

type GroupBy = "flat" | "dir" | "ext";

interface GroupedNode {
  /** Group label, e.g. "core", "rs", or empty for flat. */
  label: string;
  files: FileInfo[];
  total: number;
}

const TILE_GAP = 1;

export function CorpusTreemap({
  status,
  activeCorpusId,
  setActiveCorpusId,
  onNavigate,
}: Props) {
  void onNavigate;
  const { openEntity } = useEntityPanel();
  const corpusId = activeCorpusId ?? status.corpora[0]?.id ?? "";
  const corpus = useMemo(
    () => status.corpora.find((c) => c.id === corpusId) ?? null,
    [status.corpora, corpusId],
  );
  const [files, setFiles] = useState<FileInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [hoveredFile, setHoveredFile] = useState<FileInfo | null>(null);
  const [groupBy, setGroupBy] = useState<GroupBy>("flat");
  const [minSections, setMinSections] = useState(1);

  useEffect(() => {
    if (!corpusId) return;
    setLoading(true);
    invoke<FileInfo[]>("list_corpus_files", { corpusId })
      .then(setFiles)
      .catch(() => setFiles([]))
      .finally(() => setLoading(false));
  }, [corpusId]);

  // Reset filters when corpus changes.
  useEffect(() => {
    setMinSections(1);
    setGroupBy("flat");
    setHoveredFile(null);
  }, [corpusId]);

  const totalSections = files.reduce((s, f) => s + f.section_count, 0);
  const sortedFiles = useMemo(
    () => [...files].sort((a, b) => b.section_count - a.section_count),
    [files],
  );

  const visibleFiles = useMemo(
    () => sortedFiles.filter((f) => f.section_count >= minSections),
    [sortedFiles, minSections],
  );
  const hiddenCount = sortedFiles.length - visibleFiles.length;

  // Quartile thresholds based on the visible set, used by both the treemap
  // and the top-files list so visual rhythm stays consistent.
  const quartiles = useMemo(() => {
    if (visibleFiles.length === 0) return { q1: 0, q2: 0, q3: 0 };
    // q1 = 75th percentile, q2 = 50th, q3 = 25th.
    const counts = visibleFiles.map((f) => f.section_count).sort((a, b) => b - a);
    const at = (pct: number) =>
      counts[Math.min(counts.length - 1, Math.floor(counts.length * pct))];
    return {
      q1: at(0.25),
      q2: at(0.5),
      q3: at(0.75),
    };
  }, [visibleFiles]);

  function quartileBucket(count: number): "top" | "high" | "mid" | "low" {
    if (count >= quartiles.q1) return "top";
    if (count >= quartiles.q2) return "high";
    if (count >= quartiles.q3) return "mid";
    return "low";
  }

  const grouped = useMemo<GroupedNode[]>(() => {
    if (groupBy === "flat") {
      return [
        {
          label: "",
          files: visibleFiles,
          total: visibleFiles.reduce((s, f) => s + f.section_count, 0),
        },
      ];
    }
    const m = new Map<string, FileInfo[]>();
    for (const f of visibleFiles) {
      const key =
        groupBy === "ext"
          ? (f.path.split(".").pop() ?? "?").toLowerCase()
          : (() => {
              // Top-level directory of the path.
              const norm = f.path.replace(/\\/g, "/");
              const parts = norm.split("/").filter(Boolean);
              return parts[0] ?? "/";
            })();
      const arr = m.get(key) ?? [];
      arr.push(f);
      m.set(key, arr);
    }
    return Array.from(m.entries())
      .map(([label, arr]) => ({
        label,
        files: arr,
        total: arr.reduce((s, f) => s + f.section_count, 0),
      }))
      .sort((a, b) => b.total - a.total);
  }, [visibleFiles, groupBy]);

  const langBreakdown = useMemo(() => {
    if (visibleFiles.length === 0) return [];
    const total = visibleFiles.reduce((s, f) => s + f.section_count, 0);
    if (total === 0) return [];
    const counts = new Map<string, number>();
    for (const f of visibleFiles) {
      const ext = (f.path.split(".").pop() ?? "?").toLowerCase();
      counts.set(ext, (counts.get(ext) ?? 0) + f.section_count);
    }
    return Array.from(counts.entries())
      .map(([ext, count]) => ({
        ext,
        count,
        pct: (count / total) * 100,
      }))
      .sort((a, b) => b.count - a.count);
  }, [visibleFiles]);

  function onCellClick(f: FileInfo) {
    setActiveCorpusId(corpusId);
    openEntity({ kind: "file", corpusId, path: f.path });
  }

  return (
    <div className="@container/page flex flex-col gap-3 h-full min-h-0">
      {/* Header strip — title + corpus summary */}
      <header className="flex items-center justify-between gap-4 flex-wrap">
        <div>
          <H1 className="flex items-center gap-2">
            <TreePine className="h-5 w-5 text-text-dim" strokeWidth={2} />
            Structure
          </H1>
          <p className="font-sans text-sm text-text-dim mt-1">
            File size proportional to section count · click to drill in.
          </p>
        </div>
        <span className="font-mono text-xs tabular-nums text-text-dim">
          {visibleFiles.length} files · {totalSections.toLocaleString()} sections
        </span>
      </header>

      {/* Controls row: group-by toggle + min-sections threshold */}
      <div className="flex items-center gap-3 flex-wrap">
        <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
          Group
        </span>
        <div className="flex items-stretch gap-0">
          {(
            [
              { key: "flat" as const, label: "Flat" },
              { key: "dir" as const, label: "By dir" },
              { key: "ext" as const, label: "By ext" },
            ]
          ).map(({ key, label }) => (
            <button
              key={key}
              onClick={() => setGroupBy(key)}
              className={cn(
                "border border-border-soft px-2 py-0.5 font-sans text-sm font-medium cursor-pointer transition-colors duration-150 ease-out -ml-[1px] first:ml-0",
                groupBy === key
                  ? "border-accent bg-surface-overlay text-text z-10 relative"
                  : "bg-surface text-text-muted hover:bg-surface-overlay hover:text-text",
              )}
            >
              {label}
            </button>
          ))}
        </div>

        <span className="w-px h-5 bg-border-soft" />

        <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
          Min sections
        </span>
        <input
          type="number"
          min={0}
          value={minSections}
          onChange={(e) =>
            setMinSections(Math.max(0, parseInt(e.target.value) || 0))
          }
          className="h-7 w-16 border border-border-soft bg-surface px-2 text-sm font-mono tabular-nums text-text focus:outline-none focus:border-accent transition-colors duration-150 ease-out"
        />
        {hiddenCount > 0 && (
          <span className="font-sans text-xs text-text-dim">
            + {hiddenCount} under threshold
          </span>
        )}
      </div>

      {/* Unified component: treemap (top) · lang ribbon (middle) · top-files (bottom) */}
      <div className="border border-border-soft bg-surface flex-1 min-h-0 flex flex-col">
        {/* Treemap */}
        <div className="relative flex-1 min-h-[280px] bg-surface-sunken">
          {loading ? (
            <div className="flex items-center justify-center h-full font-sans text-base text-text-dim">
              Loading<span className="ministr-blink">_</span>
            </div>
          ) : visibleFiles.length === 0 ? (
            <div className="flex flex-col items-center justify-center gap-1.5 h-full text-center">
              <p className="font-sans text-lg font-bold text-text">
                No files indexed
              </p>
              <p className="font-sans text-sm text-text-dim">
                Kick off an ingestion run to populate this view.
              </p>
            </div>
          ) : (
            <>
              {hoveredFile && (
                <div className="absolute top-2 right-2 z-20 max-w-[340px] border border-border-soft bg-surface px-2.5 py-1.5 shadow-sm">
                  <p className="font-mono text-xs text-text break-all">
                    {corpusRelative(hoveredFile.path, corpus)}
                  </p>
                  <p className="font-mono text-mono-mini tabular-nums text-text-dim mt-0.5">
                    {hoveredFile.section_count} sections ·{" "}
                    {hoveredFile.content_hash.slice(0, 12)}
                  </p>
                </div>
              )}

              <div className="flex flex-col h-full p-1.5 gap-[2px]">
                {grouped.map((group) => (
                  <GroupBlock
                    key={group.label || "flat"}
                    group={group}
                    showLabel={groupBy !== "flat"}
                    quartileBucket={quartileBucket}
                    pathTooltip={(p) => corpusRelative(p, corpus)}
                    onHover={setHoveredFile}
                    onClick={onCellClick}
                  />
                ))}
              </div>
            </>
          )}
        </div>

        {/* Lang mix ribbon */}
        {langBreakdown.length > 0 && (
          <div className="border-t border-border">
            <div className="flex items-stretch h-6 -mx-[1px]">
              {langBreakdown.map(({ ext, pct }, i) => (
                <div
                  key={ext}
                  title={`.${ext} · ${pct.toFixed(1)}%`}
                  className={cn(
                    "border border-border-soft min-w-0 -ml-[2px] first:ml-0",
                    bucketShade(i, langBreakdown.length),
                  )}
                  style={{ width: `${Math.max(pct, 3)}%` }}
                />
              ))}
            </div>
            <div className="flex items-center gap-3 px-2 py-1 border-t border-border bg-surface-overlay overflow-x-auto">
              <span className="font-mono text-xs font-bold uppercase tracking-[0.08em] text-text-dim shrink-0">Lang mix</span>
              {langBreakdown.slice(0, 8).map(({ ext, pct }) => (
                <span
                  key={ext}
                  className="font-mono text-xs text-text-dim shrink-0"
                >
                  .{ext}{" "}
                  <span className="text-text tabular-nums">
                    {pct.toFixed(0)}%
                  </span>
                </span>
              ))}
            </div>
          </div>
        )}

        {/* Top files list */}
        <div className="border-t border-border max-h-72 overflow-y-auto">
          <div className="flex items-center justify-between border-b border-border bg-surface-overlay px-3 py-1 sticky top-0 z-10">
            <span className="font-sans text-xs font-bold tracking-[0.08em] text-text">
              Top files by sections
            </span>
            <span className="font-mono text-xs tabular-nums text-text-dim">
              {Math.min(50, visibleFiles.length)} OF {visibleFiles.length}
            </span>
          </div>
          {visibleFiles.slice(0, 50).map((f) => {
            const bucket = quartileBucket(f.section_count);
            const max = sortedFiles[0]?.section_count ?? 1;
            const pct = (f.section_count / max) * 100;
            return (
              <button
                key={f.path}
                onClick={() => onCellClick(f)}
                onMouseEnter={() => setHoveredFile(f)}
                onMouseLeave={() => setHoveredFile(null)}
                title={f.path}
                className="w-full text-left flex items-center gap-2 border-b border-border last:border-b-0 px-3 py-1 cursor-pointer transition-colors duration-150 ease-out hover:bg-surface-overlay hover:text-text"
              >
                <span
                  className={cn(
                    "h-3 w-3 border border-border-soft shrink-0",
                    bucketBg(bucket),
                  )}
                />
                <span className="font-mono text-mono-mini truncate flex-1">
                  {corpusRelative(f.path, corpus)}
                </span>
                <div className="w-20 h-1.5 border border-border-soft bg-surface-overlay overflow-hidden shrink-0">
                  <div className="h-full bg-accent" style={{ width: `${pct}%` }} />
                </div>
                <span className="font-mono text-xs tabular-nums w-12 text-right shrink-0">
                  {f.section_count}
                </span>
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}

// ─── GROUP BLOCK ──────────────────────────────────────────────────────────

function GroupBlock({
  group,
  showLabel,
  quartileBucket,
  pathTooltip,
  onHover,
  onClick,
}: {
  group: GroupedNode;
  showLabel: boolean;
  quartileBucket: (count: number) => "top" | "high" | "mid" | "low";
  /** Tooltip-formatter for cell hover. Caller passes a corpus-relative
   *  formatter so deep absolute paths don't clutter the title attribute. */
  pathTooltip: (path: string) => string;
  onHover: (f: FileInfo | null) => void;
  onClick: (f: FileInfo) => void;
}) {
  const total = group.total || 1;
  return (
    <div
      className="border border-border-soft bg-surface flex flex-col min-h-0 flex-1"
      style={{
        flexGrow: Math.max(1, group.total),
      }}
    >
      {showLabel && (
        <div className="border-b border-border bg-surface-overlay px-2 py-0.5 flex items-center justify-between">
          <span className="font-mono text-xs font-bold tracking-[0.08em] text-text">
            {group.label}
          </span>
          <span className="font-mono text-xs tabular-nums text-text-dim">
            {group.files.length} · {group.total.toLocaleString()}
          </span>
        </div>
      )}
      <div
        className="flex-1 flex flex-wrap gap-[1px] p-[2px] min-h-0"
        style={{ alignContent: "flex-start" }}
      >
        {group.files.map((f) => {
          const bucket = quartileBucket(f.section_count);
          // Side length scales with sqrt(share). Floor 6px so every file is
          // at least clickable; ceil 220px so a single dominant file doesn't
          // eat the entire viewport.
          const side = Math.max(
            6,
            Math.min(220, Math.sqrt((f.section_count / total) * 28000)),
          );
          const showLbl = side > 60;
          const basename = f.path.split(/[\\/]/).pop() ?? f.path;
          return (
            <button
              key={f.path}
              onClick={() => onClick(f)}
              onMouseEnter={() => onHover(f)}
              onMouseLeave={() => onHover(null)}
              title={`${pathTooltip(f.path)} · ${f.section_count}`}
              className={cn(
                "border border-border cursor-pointer transition-colors duration-150 ease-out hover:bg-surface-overlay hover:text-text overflow-hidden flex items-center justify-center",
                bucketBg(bucket),
                bucketText(bucket),
              )}
              style={{
                width: side,
                height: side,
                marginRight: TILE_GAP,
                marginBottom: TILE_GAP,
              }}
            >
              {showLbl && (
                <span className="font-mono text-xs font-bold tracking-[0.08em] text-center px-1 truncate w-full">
                  {basename.replace(/\.[^.]+$/, "")}
                </span>
              )}
            </button>
          );
        })}
      </div>
    </div>
  );
}

// ─── BUCKET / SHADE HELPERS ───────────────────────────────────────────────

function bucketBg(b: "top" | "high" | "mid" | "low"): string {
  switch (b) {
    case "top":
      return "bg-accent";
    case "high":
      return "bg-text";
    case "mid":
      return "bg-text-muted";
    case "low":
    default:
      return "bg-surface-overlay";
  }
}

function bucketText(b: "top" | "high" | "mid" | "low"): string {
  switch (b) {
    case "top":
      return "text-[var(--color-accent-fg-on)]";
    case "high":
      // Inverted text on the inverted body fill.
      return "text-bg";
    case "mid":
      return "text-bg";
    case "low":
    default:
      return "text-text";
  }
}

/** Lang mix ribbon — graded shading by index (top extension darkest). */
function bucketShade(i: number, total: number): string {
  if (i === 0) return "bg-accent";
  if (i <= Math.floor(total * 0.25)) return "bg-text";
  if (i <= Math.floor(total * 0.5)) return "bg-text-muted";
  return "bg-surface-overlay";
}
