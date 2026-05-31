/**
 * CodeLanding — the Code surface's no-file landing.
 *
 * Single responsibility: when nothing is open yet, orient the user in the
 * corpus and give them somewhere to click — headline stats, a language
 * breakdown, and the most substantial files as click-to-open quick-starts —
 * instead of a bare centered prompt. Derived from the already-available
 * corpus file list + CorpusInfo; no backend change.
 */
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Code2, Command, FileCode2, Layers, Hash } from "lucide-react";
import type { CorpusInfo, FileInfo } from "../../lib/types";
import { langStats } from "./langStats";

interface Props {
  corpusId: string;
  corpus: CorpusInfo | null;
  onOpen: (path: string) => void;
}

/** Last path segment, for a compact file label. */
function baseName(path: string): string {
  const segs = path.split("/").filter(Boolean);
  return segs[segs.length - 1] ?? path;
}

/** Penultimate directories, for a dimmed location subtitle. */
function dirHint(path: string): string {
  const segs = path.split("/").filter(Boolean);
  return segs.slice(0, -1).slice(-2).join("/");
}

function formatCount(n: number): string {
  return n.toLocaleString();
}

export function CodeLanding({ corpusId, corpus, onOpen }: Props) {
  const [files, setFiles] = useState<FileInfo[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!corpusId) return;
    let cancelled = false;
    setLoading(true);
    invoke<FileInfo[]>("list_corpus_files", { corpusId })
      .then((r) => {
        if (!cancelled) setFiles(r);
      })
      .catch(() => {
        if (!cancelled) setFiles([]);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [corpusId]);

  const langs = useMemo(() => langStats(files.map((f) => f.path)), [files]);
  const notable = useMemo(
    () =>
      [...files]
        .sort((a, b) => b.section_count - a.section_count)
        .slice(0, 8),
    [files],
  );

  const name = corpus?.display_name?.trim() || "this corpus";
  const fileCount = files.length || corpus?.files_indexed || 0;
  const sectionCount = corpus?.sections_count ?? 0;
  const symbolCount = corpus?.symbols_count ?? 0;

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex max-w-3xl flex-col gap-6 px-6 py-10">
        {/* Headline */}
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-2 text-accent">
            <Code2 className="h-5 w-5" strokeWidth={1.8} />
            <span className="font-mono text-xs font-bold uppercase tracking-[0.08em]">
              Code
            </span>
          </div>
          <h1 className="font-sans text-2xl font-semibold text-text">{name}</h1>
          <p className="max-w-xl font-sans text-sm text-text-dim">
            Browse the codebase through the same symbol graph the AI uses. Pick a
            file below or from the tree, or press ⌘K to jump to any symbol.
          </p>
        </div>

        {/* Stat row */}
        <div className="grid grid-cols-3 gap-3">
          <StatTile icon={FileCode2} label="Files" value={fileCount} />
          <StatTile icon={Layers} label="Sections" value={sectionCount} />
          <StatTile icon={Hash} label="Symbols" value={symbolCount} />
        </div>

        {/* Language breakdown */}
        {langs.length > 0 && (
          <section className="flex flex-col gap-3">
            <h2 className="font-sans text-xs font-semibold uppercase tracking-[0.08em] text-text-dim">
              Languages
            </h2>
            <div className="flex h-2 w-full overflow-hidden rounded-full bg-surface-sunken">
              {langs.map((l, i) => (
                <div
                  key={l.label}
                  className={i === 0 ? "bg-accent" : "bg-border"}
                  style={{ width: `${Math.max(l.fraction * 100, 1.5)}%` }}
                  title={`${l.label} · ${l.count}`}
                />
              ))}
            </div>
            <div className="flex flex-wrap gap-x-4 gap-y-1">
              {langs.map((l, i) => (
                <span key={l.label} className="flex items-center gap-1.5">
                  <span
                    className={`inline-block h-2 w-2 rounded-full ${i === 0 ? "bg-accent" : "bg-border"}`}
                  />
                  <span className="font-mono text-mono-mini text-text-muted">
                    {l.label}
                  </span>
                  <span className="font-mono text-mono-mini tabular-nums text-text-dim">
                    {l.count}
                  </span>
                </span>
              ))}
            </div>
          </section>
        )}

        {/* Notable files — quick-start */}
        <section className="flex flex-col gap-3">
          <h2 className="font-sans text-xs font-semibold uppercase tracking-[0.08em] text-text-dim">
            Jump in
          </h2>
          {loading ? (
            <p className="font-mono text-mono-mini text-text-dim">Loading_</p>
          ) : notable.length === 0 ? (
            <p className="font-mono text-mono-mini text-text-dim">
              No files indexed yet.
            </p>
          ) : (
            <div className="grid grid-cols-1 gap-2 @min-[640px]/page:grid-cols-2">
              {notable.map((f) => (
                <button
                  key={f.path}
                  type="button"
                  onClick={() => onOpen(f.path)}
                  title={f.path}
                  className="group flex items-center gap-2.5 rounded-md border border-border-soft bg-surface px-3 py-2 text-left hover:border-border hover:bg-surface-overlay cursor-pointer transition-colors duration-150 ease-out"
                >
                  <FileCode2
                    className="h-4 w-4 shrink-0 text-text-dim group-hover:text-accent transition-colors duration-150 ease-out"
                    strokeWidth={1.8}
                  />
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className="truncate font-mono text-xs text-text">
                      {baseName(f.path)}
                    </span>
                    {dirHint(f.path) && (
                      <span className="truncate font-mono text-mono-mini text-text-dim">
                        {dirHint(f.path)}
                      </span>
                    )}
                  </span>
                  <span className="shrink-0 font-mono text-mono-mini tabular-nums text-text-dim">
                    {formatCount(f.section_count)}
                  </span>
                </button>
              ))}
            </div>
          )}
        </section>

        {/* Keyboard hint footer */}
        <div className="flex items-center gap-1.5 font-mono text-mono-mini text-text-dim">
          <Command className="h-3 w-3" strokeWidth={2} />
          <span>K</span>
          <span>to jump straight to any symbol.</span>
        </div>
      </div>
    </div>
  );
}

function StatTile({
  icon: Icon,
  label,
  value,
}: {
  icon: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  label: string;
  value: number;
}) {
  return (
    <div className="flex flex-col gap-1 rounded-md border border-border-soft bg-surface px-3 py-2.5">
      <div className="flex items-center gap-1.5 text-text-dim">
        <Icon className="h-3.5 w-3.5" strokeWidth={1.8} />
        <span className="font-sans text-xs font-medium">{label}</span>
      </div>
      <span className="font-mono text-xl font-semibold tabular-nums text-text">
        {formatCount(value)}
      </span>
    </div>
  );
}
