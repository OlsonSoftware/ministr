/**
 * CodeOverview — the Explore facet's ENTRY (no file open yet).
 *
 * A command-deck CODEBASE OBSERVATORY: it answers "what is this project, and how
 * do I understand it?" and ties the Explore lenses together. A glowing identity
 * HERO (medallion + name + root + LIVE) sits over a divided vital readout
 * (Files / Sections / Symbols); a premium CODE INTELLIGENCE deck — Bridges /
 * Unused / Quality — gives each lens a live count and a one-click jump; the
 * language composition reads as an accent-ramp proportion viz; and the notable
 * files offer a quick start. Built from the v4 tokens/atoms (Medallion + glow,
 * MetricTile cell readout, StatusDot liveness). The pure `CodeOverview` renders
 * from props (Storybook); `CodeOverviewConnector` wires the live file list + the
 * three intelligence counts.
 */
import { type ReactNode, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowUpRight,
  Boxes,
  Cable,
  Code2,
  Command,
  FileCode2,
  Hash,
  Layers,
  ShieldCheck,
  Trash2,
} from "@/components/ui/icons";

import { MetricTile } from "@/components/ui/metric-tile";
import { StatusDot } from "@/components/ui/status-dot";
import type { CorpusInfo, FileInfo } from "../../lib/types";
import { cn } from "../../lib/utils";
import { type LangStat, langStats } from "./langStats";
import { CodebaseConstellation, structureModuleCount } from "./CodebaseConstellation";

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

/** A short, human root from the first corpus path (last two segments). */
function rootHint(corpus: CorpusInfo | null): string | null {
  const p = corpus?.paths?.[0];
  if (!p) return null;
  const segs = p.split("/").filter(Boolean);
  if (segs.length <= 2) return p;
  return `…/${segs.slice(-2).join("/")}`;
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
  const root = rootHint(corpus);
  const fileCount = files.length || corpus?.files_indexed || 0;
  const sectionCount = corpus?.sections_count ?? 0;
  const symbolCount = corpus?.symbols_count ?? 0;
  const sessions = corpus?.active_sessions ?? 0;
  const live = sessions > 0;

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex max-w-3xl flex-col gap-6 px-6 py-9">
        {/* ── Identity hero + divided vital readout — one command-deck panel ── */}
        <section className="flex flex-col overflow-hidden rounded-xl border border-border bg-surface-raised shadow-sm">
          <div className="flex items-start gap-3.5 px-5 pb-5 pt-5">
            <Medallion live={live} />
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-1.5 text-accent">
                <Code2 className="h-3.5 w-3.5" strokeWidth={2} />
                <span className="font-mono text-mono-micro font-bold uppercase tracking-[0.12em]">
                  Codebase
                </span>
              </div>
              <div className="mt-1 flex flex-wrap items-center gap-x-2.5 gap-y-1">
                <h1 className="truncate font-sans text-2xl font-semibold leading-tight text-text">
                  {name}
                </h1>
                {live && <LivePill count={sessions} />}
              </div>
              {root && (
                <p className="mt-1 truncate font-mono text-mono-mini text-text-dim">
                  {root}
                </p>
              )}
              <p className="mt-2.5 max-w-xl font-sans text-sm leading-relaxed text-text-dim">
                Browse the codebase through the same symbol graph the AI uses — or
                read it through a lens: its cross-language seams, what nothing
                references, and where it bends the SOLID principles.
              </p>
            </div>
          </div>

          {/* Divided vital readout — the size of the corpus at a glance. */}
          <div className="grid grid-cols-3 divide-x divide-border border-t border-border bg-surface">
            <MetricTile
              variant="cell"
              icon={FileCode2}
              label="files"
              value={fileCount.toLocaleString()}
            />
            <MetricTile
              variant="cell"
              icon={Layers}
              label="sections"
              value={sectionCount.toLocaleString()}
            />
            <MetricTile
              variant="cell"
              icon={Hash}
              label="symbols"
              value={symbolCount.toLocaleString()}
            />
          </div>
        </section>

        {/* ── Code intelligence — the premium lens deck. ── */}
        <section className="flex flex-col gap-3">
          <SectionLabel>Code intelligence</SectionLabel>
          <div className="grid grid-cols-1 gap-3 @min-[560px]/page:grid-cols-3">
            <IntelCard
              icon={Cable}
              label="Bridges"
              hint="Cross-language seams"
              count={intel.bridges}
              onClick={() => onOpenLens("bridges")}
            />
            <IntelCard
              icon={Trash2}
              label="Unused"
              hint="Dead-code candidates"
              count={intel.unused}
              tone="warning"
              onClick={() => onOpenLens("unused")}
            />
            <IntelCard
              icon={ShieldCheck}
              label="Quality"
              hint="SOLID findings"
              count={intel.quality}
              onClick={() => onOpenLens("solid")}
            />
          </div>
        </section>

        {/* ── Language composition — accent-ramp proportion viz. ── */}
        {langs.length > 0 && (
          <section className="flex flex-col gap-3">
            <SectionLabel>Languages</SectionLabel>
            <LanguageBar langs={langs} />
          </section>
        )}

        {/* ── Structure — the codebase shape as a packed module constellation. ── */}
        {structureModuleCount(files) >= 2 && (
          <section className="flex flex-col gap-3">
            <SectionLabel>Structure</SectionLabel>
            <CodebaseConstellation files={files} onOpen={onOpen} />
          </section>
        )}

        {/* ── Notable files — quick start. ── */}
        <section className="flex flex-col gap-3">
          <SectionLabel>Jump in</SectionLabel>
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
                  className="group flex items-center gap-2.5 rounded-lg border border-border-soft bg-surface px-3 py-2.5 text-left transition-colors duration-150 ease-out hover:border-accent/50 hover:bg-surface-raised cursor-pointer"
                >
                  <FileCode2
                    className="h-4 w-4 shrink-0 text-text-dim transition-colors duration-150 ease-out group-hover:text-accent"
                    strokeWidth={1.8}
                  />
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className="truncate font-mono text-xs text-text">
                      {baseName(f.path)}
                    </span>
                    {dirHint(f.path) && (
                      <span className="truncate font-mono text-mono-micro text-text-dim">
                        {dirHint(f.path)}
                      </span>
                    )}
                  </span>
                  <span className="shrink-0 rounded border border-border-soft bg-surface-overlay px-1.5 py-0.5 font-mono text-mono-micro tabular-nums text-text-dim">
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

// ───────────────────────────────────────────────────────────────────────────
// Command-deck identity helpers (ScopeHeader language, local to this surface).

/** The glowing identity medallion — the object glyph in a rounded tile, soft
 *  accent glow + a live status dot when the corpus has active sessions. */
function Medallion({ live }: { live: boolean }) {
  return (
    <span
      aria-hidden
      className={cn(
        "relative grid h-12 w-12 shrink-0 place-items-center rounded-xl border bg-surface-overlay",
        live
          ? "border-accent/50 text-accent shadow-[var(--glow-soft)]"
          : "border-border text-text-muted",
      )}
    >
      <Boxes className="h-5 w-5" strokeWidth={2} />
      <span className="absolute -right-1 -top-1 grid place-items-center rounded-full bg-surface-raised p-0.5">
        <StatusDot
          tone={live ? "accent" : "muted"}
          pulse={live ? "live" : "off"}
          size="md"
        />
      </span>
    </span>
  );
}

/** A pill announcing live agents on the corpus. Accent flavour on the
 *  border/dot; the text stays high-contrast (AA-safe). */
function LivePill({ count }: { count: number }) {
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full border border-accent/40 bg-accent/10 px-2 py-0.5">
      <StatusDot tone="accent" pulse="live" size="sm" />
      <span className="font-mono text-mono-micro font-semibold uppercase tracking-[0.08em] text-text">
        {count} live
      </span>
    </span>
  );
}

/** A deck section eyebrow. */
function SectionLabel({ children }: { children: ReactNode }) {
  return (
    <h2 className="font-mono text-mono-micro font-semibold uppercase tracking-[0.12em] text-text-dim">
      {children}
    </h2>
  );
}

// ───────────────────────────────────────────────────────────────────────────
// Code-intelligence deck card.

/** A clickable code-intelligence card — a deep-link into a lens with its live
 *  count. Clickable + keyboard-reachable even while the count is still loading;
 *  hover lifts it to the raised tier and lights the glyph. */
function IntelCard({
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
      className={cn(
        "group flex items-center gap-3 rounded-xl border border-border bg-surface px-3.5 py-3 text-left",
        "transition-all duration-150 ease-out cursor-pointer",
        "hover:-translate-y-0.5 hover:bg-surface-raised hover:shadow-sm",
        tone === "warning" ? "hover:border-warning/60" : "hover:border-accent/60",
      )}
    >
      <span
        className={cn(
          "grid h-10 w-10 shrink-0 place-items-center rounded-lg border transition-shadow duration-150",
          tone === "warning"
            ? "border-warning/40 bg-warning/10 text-warning group-hover:shadow-[var(--glow-soft)]"
            : "border-accent/40 bg-accent/10 text-accent group-hover:shadow-[var(--glow-soft)]",
        )}
      >
        <Icon className="h-[18px] w-[18px]" strokeWidth={2} />
      </span>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="font-mono text-2xl font-semibold leading-none tabular-nums text-text">
          {count === null ? <span className="text-text-dim">··</span> : count.toLocaleString()}
        </span>
        <span className="mt-1.5 font-sans text-xs font-semibold text-text">
          {label}
        </span>
        <span className="truncate font-mono text-mono-micro text-text-dim">
          {hint}
        </span>
      </span>
      <ArrowUpRight
        className="h-4 w-4 shrink-0 text-text-dim transition-colors duration-150 group-hover:text-accent"
        strokeWidth={2.25}
      />
    </button>
  );
}

// ───────────────────────────────────────────────────────────────────────────
// Language composition — an accent-ramp proportion viz.

/** Descending accent-opacity ramp — the dominant language is full accent, the
 *  rest fade down a step at a time. Stays inside the single-accent system (no
 *  per-language hues); "Other" is the neutral border grey. */
const ACCENT_RAMP = [
  "bg-accent",
  "bg-accent/70",
  "bg-accent/55",
  "bg-accent/40",
  "bg-accent/30",
  "bg-accent/25",
] as const;

function langColor(label: string, rank: number): string {
  if (label === "Other") return "bg-border";
  return ACCENT_RAMP[Math.min(rank, ACCENT_RAMP.length - 1)];
}

function LanguageBar({ langs }: { langs: LangStat[] }) {
  return (
    <div className="flex flex-col gap-3">
      {/* Segmented proportion bar — gap-px lets the sunken track show through
          as crisp dividers; the rounded container clips the ends. */}
      <div className="flex h-3 w-full gap-px overflow-hidden rounded-full bg-surface-sunken">
        {langs.map((l, i) => (
          <div
            key={l.label}
            className={langColor(l.label, i)}
            style={{ width: `${Math.max(l.fraction * 100, 1.5)}%` }}
            title={`${l.label} · ${l.count}`}
          />
        ))}
      </div>
      {/* Legend — swatch · language · percent · count. */}
      <div className="flex flex-wrap gap-x-4 gap-y-1.5">
        {langs.map((l, i) => {
          const pct = Math.round(l.fraction * 100);
          return (
            <span key={l.label} className="flex items-center gap-1.5">
              <span
                className={cn(
                  "inline-block h-2 w-2 shrink-0 rounded-full",
                  langColor(l.label, i),
                )}
              />
              <span className="font-mono text-mono-mini text-text-muted">
                {l.label}
              </span>
              <span className="font-mono text-mono-mini tabular-nums text-text-dim">
                {pct >= 1 ? `${pct}%` : `${l.count}`}
              </span>
            </span>
          );
        })}
      </div>
    </div>
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
