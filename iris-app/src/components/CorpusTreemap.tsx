import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TreePine, FileCode, File as FileIcon, Layers } from "lucide-react";
import { Card } from "./ui/card";
import { Badge } from "./ui/badge";
import { cn } from "../lib/utils";
import type { DaemonStatus, FileInfo } from "../lib/types";

interface Props {
  status: DaemonStatus;
}

const LANG_COLORS: Record<string, string> = {
  rs: "bg-orange-500",
  ts: "bg-blue-500",
  tsx: "bg-sky-400",
  js: "bg-yellow-400",
  jsx: "bg-amber-400",
  py: "bg-emerald-500",
  go: "bg-cyan-500",
  java: "bg-red-500",
  kt: "bg-violet-500",
  swift: "bg-orange-400",
  c: "bg-slate-500",
  cpp: "bg-indigo-500",
  cs: "bg-purple-500",
  rb: "bg-rose-500",
  md: "bg-accent",
  toml: "bg-red-400",
  json: "bg-zinc-500",
  yaml: "bg-pink-400",
  yml: "bg-pink-400",
};

export function CorpusTreemap({ status }: Props) {
  const [corpusId, setCorpusId] = useState(status.corpora[0]?.id ?? "");
  const [files, setFiles] = useState<FileInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [hoveredFile, setHoveredFile] = useState<FileInfo | null>(null);

  useEffect(() => {
    if (!corpusId) return;
    setLoading(true);
    invoke<FileInfo[]>("list_corpus_files", { corpusId })
      .then(setFiles)
      .catch(() => setFiles([]))
      .finally(() => setLoading(false));
  }, [corpusId]);

  const totalSections = files.reduce((s, f) => s + f.section_count, 0);
  const langBreakdown = getLangBreakdown(files);
  const sortedFiles = [...files].sort((a, b) => b.section_count - a.section_count);

  return (
    <div className="space-y-4 iris-fade-in">
      <header className="flex items-end justify-between gap-4 flex-wrap">
        <div>
          <h2 className="text-base font-semibold text-text flex items-center gap-2">
            <TreePine className="h-4 w-4 text-accent" />
            Corpus treemap
          </h2>
          <p className="text-xs text-text-dim mt-0.5">
            File size proportional to section count — hover or click rows for details.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <select
            value={corpusId}
            onChange={(e) => setCorpusId(e.target.value)}
            className="h-8 rounded-md border border-border/70 bg-surface-raised px-2.5 text-xs font-mono text-text cursor-pointer focus:outline-none focus:border-[var(--color-accent-ring)] focus:shadow-[0_0_0_3px_var(--color-accent-soft)]"
          >
            {status.corpora.map((c) => (
              <option key={c.id} value={c.id}>
                {c.id}
              </option>
            ))}
          </select>
          <Badge variant="muted">
            {files.length} files · {totalSections.toLocaleString()} sections
          </Badge>
        </div>
      </header>

      {langBreakdown.length > 0 && (
        <Card hover="lift" className="p-3">
          <div className="flex items-center gap-1.5 mb-2.5">
            <Layers className="h-3 w-3 text-text-dim" />
            <h3 className="text-[11px] font-semibold uppercase tracking-wider text-text-dim">
              Language mix
            </h3>
          </div>
          <div className="flex flex-wrap gap-x-3 gap-y-1.5">
            {langBreakdown.map(({ ext, count, pct }) => (
              <div key={ext} className="flex items-center gap-1.5 text-xs">
                <div
                  className={cn(
                    "h-2.5 w-2.5 rounded-sm shrink-0",
                    LANG_COLORS[ext] ?? "bg-zinc-500",
                  )}
                />
                <span className="font-mono text-text">.{ext}</span>
                <span className="text-text-dim tabular-nums">
                  ({count} · {pct}%)
                </span>
              </div>
            ))}
          </div>
        </Card>
      )}

      <Card hover="lift" className="p-3 relative">
        {hoveredFile && (
          <div className="absolute top-3 right-3 z-10 rounded-lg border border-border/70 bg-surface-raised px-3 py-2 text-xs max-w-[340px] shadow-[var(--shadow-md)] iris-fade-in">
            <p className="font-mono truncate text-text">{hoveredFile.path}</p>
            <p className="text-text-dim mt-1 flex items-center gap-3">
              <span className="tabular-nums">
                {hoveredFile.section_count} sections
              </span>
              <span className="font-mono">
                {hoveredFile.content_hash.slice(0, 12)}…
              </span>
            </p>
          </div>
        )}

        {loading ? (
          <div className="flex items-center justify-center py-12">
            <div className="iris-spin h-6 w-6 rounded-full border-2 border-border border-t-accent" />
          </div>
        ) : files.length === 0 ? (
          <div className="flex flex-col items-center gap-2 py-10 text-center">
            <p className="text-sm font-medium text-text">No files indexed</p>
            <p className="text-xs text-text-dim">
              Kick off an ingestion run to populate this view.
            </p>
          </div>
        ) : (
          <div className="flex flex-wrap gap-[2px] bg-surface-sunken border border-border/50 rounded-lg overflow-hidden p-1.5 min-h-[220px]">
            {sortedFiles.map((f, i) => {
              const ext = f.path.split(".").pop() ?? "";
              const area =
                totalSections > 0
                  ? Math.max(
                      6,
                      Math.sqrt((f.section_count / totalSections) * 60000),
                    )
                  : 10;
              return (
                <div
                  key={`${f.path}-${i}`}
                  className={cn(
                    "rounded-sm opacity-75 hover:opacity-100 hover:scale-105 transition-all duration-100 cursor-pointer",
                    LANG_COLORS[ext] ?? "bg-zinc-500",
                  )}
                  style={{ width: `${area}px`, height: `${area}px` }}
                  title={`${f.path} (${f.section_count} sections)`}
                  onMouseEnter={() => setHoveredFile(f)}
                  onMouseLeave={() => setHoveredFile(null)}
                />
              );
            })}
          </div>
        )}
      </Card>

      <Card hover="lift" className="p-0 overflow-hidden">
        <div className="flex items-center justify-between px-4 py-2.5 border-b border-border/60">
          <h3 className="text-[11px] font-semibold uppercase tracking-wider text-text-dim">
            Top 50 files by sections
          </h3>
          <span className="text-[11px] text-text-dim font-mono tabular-nums">
            {sortedFiles.length > 50 ? "50" : sortedFiles.length} of {sortedFiles.length}
          </span>
        </div>
        <div className="max-h-96 overflow-y-auto">
          <table className="w-full text-xs">
            <tbody>
              {sortedFiles.slice(0, 50).map((f, i) => {
                const ext = f.path.split(".").pop() ?? "";
                const maxSections = sortedFiles[0]?.section_count ?? 1;
                const pct = (f.section_count / maxSections) * 100;
                return (
                  <tr
                    key={`${f.path}-${i}`}
                    className="border-t border-border/40 first:border-0 hover:bg-surface-overlay/50"
                  >
                    <td className="py-1.5 px-4 font-mono w-full">
                      <div className="flex items-center gap-2">
                        <div
                          className={cn(
                            "h-2 w-2 rounded-sm shrink-0",
                            LANG_COLORS[ext] ?? "bg-zinc-500",
                          )}
                        />
                        {LANG_COLORS[ext] ? (
                          <FileCode className="h-3 w-3 text-text-dim shrink-0" />
                        ) : (
                          <FileIcon className="h-3 w-3 text-text-dim shrink-0" />
                        )}
                        <span className="truncate text-text">{f.path}</span>
                      </div>
                    </td>
                    <td className="py-1.5 px-3 text-right">
                      <div className="flex items-center justify-end gap-2">
                        <div className="w-16 h-1 rounded-full bg-surface-overlay overflow-hidden">
                          <div
                            className={cn(
                              "h-full",
                              LANG_COLORS[ext] ?? "bg-zinc-500",
                            )}
                            style={{ width: `${pct}%` }}
                          />
                        </div>
                        <span className="text-text-muted tabular-nums font-mono w-10">
                          {f.section_count}
                        </span>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </Card>
    </div>
  );
}

function getLangBreakdown(files: FileInfo[]) {
  const counts: Record<string, number> = {};
  for (const f of files) {
    const ext = f.path.split(".").pop() ?? "?";
    counts[ext] = (counts[ext] ?? 0) + 1;
  }
  const total = files.length || 1;
  return Object.entries(counts)
    .sort((a, b) => b[1] - a[1])
    .map(([ext, count]) => ({
      ext,
      count,
      pct: Math.round((count / total) * 100),
    }));
}
