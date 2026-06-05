/**
 * CodeOverview — the Explore facet's ENTRY (no file open yet).
 *
 * Where the old landing was a file-picker, this is a CODEBASE OVERVIEW: it
 * answers "what is this project, and how do I understand it?" and ties the four
 * Explore lenses together. Identity + size, language composition, and a CODE
 * INTELLIGENCE row — Bridges / Unused / Quality — where each tile shows a live
 * count and is a one-click jump into that lens. Below, the notable files for a
 * quick start. Built fresh from the v4 tokens/atoms; the pure `CodeOverview`
 * renders from props (Storybook) and `CodeOverviewConnector` wires the live
 * file list + the three intelligence counts.
 */
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowUpRight,
  Cable,
  Code2,
  Command,
  FileCode2,
  Hash,
  Layers,
  ShieldCheck,
  Trash2,
} from "@/components/ui/icons";

import type { CorpusInfo, FileInfo } from "../../lib/types";
import { cn } from "../../lib/utils";
import { langStats } from "./langStats";

/** The three lenses an intelligence tile can jump to. */
export type IntelLens = "bridges" | "unused" | "solid";

export interface CodeIntel {
  /** Cross-language bridge count, or null while loading. */
  bridges: number | null;
  /** Dead-code candidate count, or null while loading. */
  unused: number | null;
  /** SOLID-finding count, or null while loading. */
  quality: number | null;
}

export interface CodeOverviewProps {
  corpus: CorpusInfo | null;
  files: FileInfo[];
  filesLoading?: boolean;
  intel: CodeIntel;
  /** Open a file in the code lens. */
  onOpen: (path: string) => void;
  /** Jump to a lens (the intelligence tiles). */
  onOpenLens: (lens: IntelLens) => void;
}

function baseName(path: string): string {
  const segs = path.split("/").filter(Boolean);
  return segs[segs.length - 1] ?? path;
}

function dirHint(path: string): string {
  const segs = path.split("/").filter(Boolean);
  return segs.slice(0, -1).slice(-2).join("/");
}

export function CodeOverview({
  corpus,
  files,
  filesLoading = false,
  intel,
  onOpen,
  onOpenLens,
}: CodeOverviewProps) {
  const langs = useMemo(() => langStats(files.map((f) => f.path)), [files]);
  const notable = useMemo(
    () => [...files].sort((a, b) => b.section_count - a.section_count).slice(0, 8),
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
            Browse the codebase through the same symbol graph the AI uses — or
            read it through a lens: its cross-language seams, what nothing
            references, and where it bends the SOLID principles.
          </p>
        </div>

        {/* Size */}
        <div className="grid grid-cols-3 gap-3">
          <StatTile icon={FileCode2} label="Files" value={fileCount} />
          <StatTile icon={Layers} label="Sections" value={sectionCount} />
          <StatTile icon={Hash} label="Symbols" value={symbolCount} />
        </div>

        {/* Code intelligence — the lens deep-links. */}
        <section className="flex flex-col gap-3">
          <h2 className="font-sans text-xs font-semibold uppercase tracking-[0.08em] text-text-dim">
            Code intelligence
          </h2>
          <div className="grid grid-cols-1 gap-3 @min-[560px]/page:grid-cols-3">
            <IntelTile
              icon={Cable}
              label="Bridges"
              hint="Cross-language seams"
              count={intel.bridges}
              onClick={() => onOpenLens("bridges")}
            />
            <IntelTile
              icon={Trash2}
              label="Unused"
              hint="Dead-code candidates"
              count={intel.unused}
              tone="warning"
              onClick={() => onOpenLens("unused")}
            />
            <IntelTile
              icon={ShieldCheck}
              label="Quality"
              hint="SOLID findings"
              count={intel.quality}
              onClick={() => onOpenLens("solid")}
            />
          </div>
        </section>

        {/* Languages */}
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

        {/* Notable files — quick start */}
        <section className="flex flex-col gap-3">
          <h2 className="font-sans text-xs font-semibold uppercase tracking-[0.08em] text-text-dim">
            Jump in
          </h2>
          {filesLoading ? (
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
                    {f.section_count.toLocaleString()}
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
  icon: typeof FileCode2;
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
        {value.toLocaleString()}
      </span>
    </div>
  );
}

/** A clickable code-intelligence tile — a deep-link into a lens with its live
 *  count. Clickable even while the count is still loading. */
function IntelTile({
  icon: Icon,
  label,
  hint,
  count,
  tone = "accent",
  onClick,
}: {
  icon: typeof Cable;
  label: string;
  hint: string;
  count: number | null;
  tone?: "accent" | "warning";
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={`${label} — ${hint}`}
      className="group flex items-center gap-3 rounded-lg border border-border-soft bg-surface px-3 py-3 text-left hover:border-accent hover:bg-surface-overlay cursor-pointer transition-colors duration-150 ease-out"
    >
      <span
        className={cn(
          "grid h-9 w-9 shrink-0 place-items-center rounded-md border",
          tone === "warning"
            ? "border-warning/40 bg-warning/10 text-warning"
            : "border-accent/40 bg-accent/10 text-accent",
        )}
      >
        <Icon className="h-4 w-4" strokeWidth={2} />
      </span>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="font-mono text-lg font-semibold tabular-nums leading-none text-text">
          {count === null ? (
            <span className="text-text-dim">·· </span>
          ) : (
            count.toLocaleString()
          )}
        </span>
        <span className="mt-1 font-sans text-xs font-medium text-text">
          {label}
        </span>
        <span className="truncate font-mono text-mono-micro text-text-dim">
          {hint}
        </span>
      </span>
      <ArrowUpRight
        className="h-3.5 w-3.5 shrink-0 text-text-dim group-hover:text-accent transition-colors duration-150"
        strokeWidth={2.25}
      />
    </button>
  );
}

// ───────────────────────────────────────────────────────────────────────────
// Connector — the live file list + the three intelligence counts.

export function CodeOverviewConnector({
  corpusId,
  corpus,
  onOpen,
  onOpenLens,
}: {
  corpusId: string;
  corpus: CorpusInfo | null;
  onOpen: (path: string) => void;
  onOpenLens: (lens: IntelLens) => void;
}) {
  const [files, setFiles] = useState<FileInfo[] | null>(null);
  const [intel, setIntel] = useState<CodeIntel>({
    bridges: null,
    unused: null,
    quality: null,
  });

  useEffect(() => {
    let cancelled = false;
    setFiles(null);
    setIntel({ bridges: null, unused: null, quality: null });

    invoke<FileInfo[]>("list_corpus_files", { corpusId })
      .then((r) => !cancelled && setFiles(r))
      .catch(() => !cancelled && setFiles([]));

    // The three intelligence counts resolve independently so a slow/failed one
    // never blocks the others; a failure degrades the tile to 0 (still clickable).
    const count = (key: keyof CodeIntel, promise: Promise<unknown[]>) =>
      promise
        .then((r) => !cancelled && setIntel((p) => ({ ...p, [key]: r.length })))
        .catch(() => !cancelled && setIntel((p) => ({ ...p, [key]: 0 })));

    count(
      "bridges",
      invoke<unknown[]>("bridge_query", {
        corpusId,
        query: null,
        kind: null,
        sourceLanguage: null,
        filePath: null,
        limit: 1000,
      }),
    );
    count(
      "unused",
      invoke<unknown[]>("dead_code", {
        corpusId,
        kind: null,
        module: null,
        minLines: null,
        limit: 1000,
      }),
    );
    count("quality", invoke<unknown[]>("solid_findings", { corpusId, limit: 1000 }));

    return () => {
      cancelled = true;
    };
  }, [corpusId]);

  return (
    <CodeOverview
      corpus={corpus}
      files={files ?? []}
      filesLoading={files === null}
      intel={intel}
      onOpen={onOpen}
      onOpenLens={onOpenLens}
    />
  );
}
