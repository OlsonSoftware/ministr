import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TreePine, FileCode, File } from "lucide-react";
import { Card } from "./ui/card";
import type { DaemonStatus, FileInfo } from "../lib/types";

interface Props {
  status: DaemonStatus;
}

const LANG_COLORS: Record<string, string> = {
  rs: "bg-orange-500",
  ts: "bg-blue-500",
  tsx: "bg-blue-400",
  js: "bg-yellow-500",
  py: "bg-green-500",
  md: "bg-purple-500",
  toml: "bg-red-400",
  json: "bg-gray-500",
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

  return (
    <div className="space-y-3">
      <h2 className="text-sm font-medium text-text-muted uppercase tracking-wider flex items-center gap-2">
        <TreePine className="h-4 w-4" /> Corpus Treemap
      </h2>

      <div className="flex items-center gap-2">
        <select
          value={corpusId}
          onChange={(e) => setCorpusId(e.target.value)}
          className="text-xs bg-surface-raised border border-border rounded px-2 py-1.5"
        >
          {status.corpora.map((c) => (
            <option key={c.id} value={c.id}>
              {c.id}
            </option>
          ))}
        </select>
        <span className="text-xs text-text-dim">
          {files.length} files · {totalSections} sections
        </span>
      </div>

      {/* Language legend */}
      <div className="flex flex-wrap gap-2">
        {langBreakdown.map(({ ext, count, pct }) => (
          <div key={ext} className="flex items-center gap-1 text-xs">
            <div className={`h-2.5 w-2.5 rounded-sm ${LANG_COLORS[ext] ?? "bg-gray-400"}`} />
            <span className="text-text-dim">
              .{ext} ({count}, {pct}%)
            </span>
          </div>
        ))}
      </div>

      {/* Treemap grid */}
      {loading ? (
        <p className="text-sm text-text-dim">Loading...</p>
      ) : (
        <div className="relative">
          {hoveredFile && (
            <Card className="absolute top-0 right-0 z-10 p-2 text-xs max-w-[300px]">
              <p className="font-mono truncate">{hoveredFile.path}</p>
              <p className="text-text-dim">{hoveredFile.section_count} sections</p>
              <p className="text-text-dim">Hash: {hoveredFile.content_hash.slice(0, 12)}...</p>
            </Card>
          )}

          <div className="flex flex-wrap gap-px bg-surface-overlay rounded overflow-hidden">
            {files
              .sort((a, b) => b.section_count - a.section_count)
              .map((f, i) => {
                const ext = f.path.split(".").pop() ?? "";
                // Area proportional to section count (min 4px for visibility)
                const area = totalSections > 0
                  ? Math.max(4, Math.sqrt((f.section_count / totalSections) * 40000))
                  : 8;
                return (
                  <div
                    key={`${f.path}-${i}`}
                    className={`${LANG_COLORS[ext] ?? "bg-gray-400"} opacity-70 hover:opacity-100 transition-opacity cursor-pointer rounded-sm`}
                    style={{ width: `${area}px`, height: `${area}px` }}
                    title={`${f.path} (${f.section_count} sections)`}
                    onMouseEnter={() => setHoveredFile(f)}
                    onMouseLeave={() => setHoveredFile(null)}
                  />
                );
              })}
          </div>
        </div>
      )}

      {/* File table */}
      <div className="max-h-64 overflow-y-auto">
        <table className="w-full text-xs">
          <thead className="sticky top-0 bg-surface">
            <tr className="text-left text-text-dim">
              <th className="py-1 px-2">File</th>
              <th className="py-1 px-2 text-right">Sections</th>
            </tr>
          </thead>
          <tbody>
            {files
              .sort((a, b) => b.section_count - a.section_count)
              .slice(0, 50)
              .map((f, i) => {
                const ext = f.path.split(".").pop() ?? "";
                return (
                  <tr key={`${f.path}-${i}`} className="border-t border-border/50 hover:bg-surface-overlay">
                    <td className="py-1 px-2 font-mono flex items-center gap-1.5">
                      {LANG_COLORS[ext] ? (
                        <FileCode className="h-3 w-3 text-text-dim shrink-0" />
                      ) : (
                        <File className="h-3 w-3 text-text-dim shrink-0" />
                      )}
                      <span className="truncate">{f.path}</span>
                    </td>
                    <td className="py-1 px-2 text-right text-text-dim">{f.section_count}</td>
                  </tr>
                );
              })}
          </tbody>
        </table>
      </div>
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
    .map(([ext, count]) => ({ ext, count, pct: Math.round((count / total) * 100) }));
}
