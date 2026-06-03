/**
 * SymbolNeighborhood — the bespoke code-graph "neighborhood" peek (aaa-explore).
 *
 * The old SymbolPeek answered one question — "what is this symbol?" — with a
 * bare definition. A symbol is never alone, though: it has a NEIGHBORHOOD. This
 * peek answers the question the user actually has when they stop on a symbol —
 * "what's most relevant AROUND this?" — by fusing the code graph into one
 * navigable card:
 *
 *   · DEFINITION   — signature, kind/visibility, location, doc, source.
 *   · MOST-RELEVANT FILES — the files that touch this symbol, ranked by edge
 *                    count (the user's 'most relevant files' ask).
 *   · NEIGHBORS    — the symbols connected to it, grouped by edge kind
 *                    (callers · implementors · importers · cross-language
 *                    bridges), each click-to-navigate (the 'most relevant
 *                    symbols' ask).
 *
 * v1 is built entirely on EXISTING daemon data (symbol_definition +
 * symbol_references — incoming edges where this symbol is the target); the
 * semantic blend (embedding-similar symbols + ranked relevance) is the
 * fl-symbol-neighborhood backend deepening. Built fresh from v4 tokens/atoms;
 * the pure `SymbolNeighborhood` renders from props for Storybook and
 * `SymbolNeighborhoodConnector` wires the live invokes.
 */
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowUpRight,
  Boxes,
  Cable,
  ChevronRight,
  FileCode2,
  GitFork,
  Layers,
  Link2,
  ListTree,
  PackageOpen,
  PanelRight,
  Quote,
  Sparkles,
  X,
} from "lucide-react";

import type {
  SearchResult,
  SymbolDefinitionDetail,
  SymbolInfo,
  SymbolRef,
} from "../../lib/types";
import { cn } from "../../lib/utils";
import { useEntityPanel } from "../../hooks/useEntityPanel";
import { useWorkspaceOptional } from "../workspace/WorkspaceContext";
import { Badge } from "../ui/badge";

export interface SymbolNeighborhoodProps {
  symbolName: string;
  /** The symbol's own definition; null while loading or not found. */
  definition: SymbolDefinitionDetail | null;
  /** Incoming edges (callers/implementors/importers/bridges) — symbol is target. */
  references: SymbolRef[];
  loading?: boolean;
  /** Embedded inside a host that provides its own chrome (the EntityPanel) —
   *  hides this component's header + close so there's no double chrome. */
  embedded?: boolean;
  /** Other symbols defined in the same file (optional — the deep inspector). */
  sameFile?: SymbolInfo[];
  /** Semantic mentions of this symbol across the corpus (optional). */
  mentions?: SearchResult[];
  onClose?: () => void;
  onGoToDefinition: (filePath: string, line: number) => void;
  onJumpRef: (ref: SymbolRef) => void;
  /** Open another symbol (same-file rows) in the host inspector. */
  onOpenSymbol?: (symbol: SymbolInfo) => void;
  /** Open a mentioned section in the host inspector. */
  onOpenSection?: (result: SearchResult) => void;
  /** Escalate this symbol into the shared EntityPanel (the Explore peek). */
  onInspect?: () => void;
  /** Drop this symbol into the Ask thread + jump to the Ask facet
   *  (cross-facet OOUX — aaa-explore-integrated). */
  onAsk?: () => void;
}

// ── Edge-kind → group meta (ordered: the strongest relationships first) ──────
const GROUPS: Array<{
  kind: string;
  label: string;
  icon: typeof GitFork;
  accent: boolean;
}> = [
  { kind: "calls", label: "Callers", icon: GitFork, accent: true },
  { kind: "implements", label: "Implementors", icon: Boxes, accent: false },
  { kind: "imports", label: "Importers", icon: PackageOpen, accent: false },
  { kind: "bridge", label: "Cross-language", icon: Cable, accent: true },
  { kind: "uses", label: "Uses", icon: Link2, accent: false },
];

function fileName(path: string): { name: string; parent: string } {
  const segs = path.split("/").filter(Boolean);
  return {
    name: segs[segs.length - 1] ?? path,
    parent: segs.slice(0, -1).slice(-2).join("/"),
  };
}

export function SymbolNeighborhood({
  symbolName,
  definition,
  references,
  loading = false,
  embedded = false,
  sameFile,
  mentions,
  onClose,
  onGoToDefinition,
  onJumpRef,
  onOpenSymbol,
  onOpenSection,
  onInspect,
  onAsk,
}: SymbolNeighborhoodProps) {
  const [showSource, setShowSource] = useState(false);

  // Group the incoming edges by kind (only non-empty groups, in GROUPS order).
  const groups = useMemo(() => {
    const byKind = new Map<string, SymbolRef[]>();
    for (const r of references) {
      const k = r.ref_kind || "uses";
      const arr = byKind.get(k);
      if (arr) arr.push(r);
      else byKind.set(k, [r]);
    }
    return GROUPS.map((g) => ({ ...g, refs: byKind.get(g.kind) ?? [] })).filter(
      (g) => g.refs.length > 0,
    );
  }, [references]);

  // Most-relevant files = distinct referencing files, ranked by edge count.
  const files = useMemo(() => {
    const byFile = new Map<string, number>();
    for (const r of references) {
      if (definition && r.from_file === definition.file_path) continue; // skip self
      byFile.set(r.from_file, (byFile.get(r.from_file) ?? 0) + 1);
    }
    return [...byFile.entries()]
      .map(([path, count]) => ({ path, count }))
      .sort((a, b) => b.count - a.count || a.path.localeCompare(b.path));
  }, [references, definition]);

  const totalEdges = references.length;

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* ── Header (host provides chrome when embedded) ──────────────────── */}
      {!embedded && (
        <header className="flex items-center justify-between gap-2 border-b border-border-soft bg-surface-overlay px-3 py-2">
          <div className="flex min-w-0 items-center gap-2">
            <ListTree className="h-3.5 w-3.5 shrink-0 text-accent" strokeWidth={2.25} />
            <span className="font-mono text-xs font-bold uppercase tracking-[0.08em] text-accent">
              Neighborhood
            </span>
            <span className="truncate font-mono text-xs font-semibold text-text">
              {symbolName}
            </span>
            {definition && <Badge variant="muted">{definition.kind}</Badge>}
          </div>
          {onClose && (
            <button
              type="button"
              onClick={onClose}
              aria-label="Close neighborhood"
              className="grid h-5 w-5 shrink-0 place-items-center rounded-md border border-border-soft text-text-muted hover:border-border hover:text-text cursor-pointer transition-colors duration-150 ease-out"
            >
              <X className="h-2.5 w-2.5" strokeWidth={2} />
            </button>
          )}
        </header>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {loading && !definition ? (
          <p className="px-3 py-2 font-mono text-mono-mini text-text-dim">
            Mapping neighborhood_
          </p>
        ) : (
          <>
            {/* ── Definition ──────────────────────────────────────────── */}
            {definition && (
              <section className="border-b border-border-soft p-3 space-y-2">
                <div className="font-mono text-xs font-bold text-text break-words">
                  {definition.signature}
                </div>
                <div className="flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-mono-mini text-text-dim">
                  {definition.visibility && (
                    <span className="uppercase tracking-[0.08em]">
                      {definition.visibility}
                    </span>
                  )}
                  <span className="truncate">
                    {definition.file_path}:{definition.line_start}
                  </span>
                </div>
                {definition.doc_comment && (
                  <p className="border-l border-accent bg-surface-overlay px-2 py-1.5 font-mono text-mono-mini text-text-muted whitespace-pre-wrap">
                    {definition.doc_comment}
                  </p>
                )}

                <div className="flex flex-wrap items-center gap-2 pt-0.5">
                  <button
                    type="button"
                    onClick={() =>
                      onGoToDefinition(definition.file_path, definition.line_start)
                    }
                    className="inline-flex items-center gap-1 rounded-md border border-border bg-surface px-2 py-1 font-mono text-mono-mini font-bold uppercase tracking-[0.08em] text-text hover:bg-surface-overlay cursor-pointer transition-colors duration-150 ease-out"
                  >
                    <ArrowUpRight className="h-3 w-3" strokeWidth={2.5} />
                    Go to definition
                  </button>
                  {onAsk && (
                    <button
                      type="button"
                      onClick={onAsk}
                      title="Drop this symbol into Ask and start a conversation"
                      className="inline-flex items-center gap-1 rounded-md border border-accent/50 bg-accent/10 px-2 py-1 font-mono text-mono-mini font-bold uppercase tracking-[0.08em] text-accent hover:bg-accent/20 cursor-pointer transition-colors duration-150 ease-out"
                    >
                      <Sparkles className="h-3 w-3" strokeWidth={2.5} />
                      Ask about this
                    </button>
                  )}
                  {onInspect && (
                    <button
                      type="button"
                      onClick={onInspect}
                      title="Open in the shared inspector"
                      className="inline-flex items-center gap-1 rounded-md border border-border-soft px-2 py-1 font-mono text-mono-mini uppercase tracking-[0.08em] text-text-muted hover:border-accent hover:text-text cursor-pointer transition-colors duration-150 ease-out"
                    >
                      <PanelRight className="h-3 w-3" strokeWidth={2.5} />
                      Inspect
                    </button>
                  )}
                  {definition.source_context && (
                    <button
                      type="button"
                      onClick={() => setShowSource((s) => !s)}
                      aria-expanded={showSource}
                      className="inline-flex items-center gap-1 rounded-md border border-border-soft px-2 py-1 font-mono text-mono-mini uppercase tracking-[0.08em] text-text-muted hover:border-border hover:text-text cursor-pointer transition-colors duration-150 ease-out"
                    >
                      <ChevronRight
                        className={cn(
                          "h-3 w-3 transition-transform duration-150",
                          showSource && "rotate-90",
                        )}
                        strokeWidth={2.5}
                      />
                      Source
                    </button>
                  )}
                </div>

                {showSource && definition.source_context && (
                  <pre className="max-h-64 overflow-auto rounded-md border border-border-soft bg-surface-sunken p-2 font-mono text-mono-mini leading-relaxed text-text whitespace-pre">
                    {definition.source_context}
                  </pre>
                )}
              </section>
            )}

            {/* ── At a glance ─────────────────────────────────────────── */}
            {totalEdges > 0 && (
              <div className="flex flex-wrap items-center gap-1.5 border-b border-border-soft bg-surface px-3 py-2">
                <GlanceChip label="edges" value={totalEdges} accent />
                <GlanceChip label="files" value={files.length} />
                {groups.map((g) => (
                  <GlanceChip key={g.kind} label={g.label} value={g.refs.length} />
                ))}
              </div>
            )}

            {/* ── Most-relevant files ─────────────────────────────────── */}
            {files.length > 0 && (
              <NeighborSection icon={FileCode2} label="Most-relevant files" count={files.length}>
                {files.map((f) => {
                  const { name, parent } = fileName(f.path);
                  return (
                    <button
                      key={f.path}
                      type="button"
                      title={f.path}
                      onClick={() =>
                        onJumpRef({
                          from_name: name,
                          from_file: f.path,
                          to_name: symbolName,
                          to_file: definition?.file_path ?? "",
                          ref_kind: "uses",
                        })
                      }
                      className="group flex w-full items-center gap-2 px-3 py-1.5 text-left font-mono text-mono-mini text-text-muted hover:bg-surface-overlay hover:text-text cursor-pointer transition-colors duration-150 ease-out"
                    >
                      <FileCode2 className="h-3 w-3 shrink-0 text-text-dim group-hover:text-accent" strokeWidth={2} />
                      <span className="flex min-w-0 flex-1 flex-col">
                        <span className="truncate text-text">{name}</span>
                        {parent && <span className="truncate text-text-dim">{parent}</span>}
                      </span>
                      <span className="shrink-0 tabular-nums text-text-dim">{f.count}</span>
                    </button>
                  );
                })}
              </NeighborSection>
            )}

            {/* ── Neighbor symbols, grouped by edge kind ──────────────── */}
            {groups.map((g) => (
              <NeighborSection key={g.kind} icon={g.icon} label={g.label} count={g.refs.length} accent={g.accent}>
                {g.refs.map((r, i) => {
                  const { name, parent } = fileName(r.from_file);
                  return (
                    <button
                      key={`${r.from_file}:${r.from_name}:${i}`}
                      type="button"
                      title={`${r.from_name} · ${r.from_file}`}
                      onClick={() => onJumpRef(r)}
                      className="group flex w-full items-center gap-2 px-3 py-1.5 text-left font-mono text-mono-mini text-text-muted hover:bg-surface-overlay hover:text-text cursor-pointer transition-colors duration-150 ease-out"
                    >
                      <ChevronRight className="h-3 w-3 shrink-0 text-text-dim group-hover:text-accent" strokeWidth={2.5} />
                      <span className="flex min-w-0 flex-1 flex-col">
                        <span className="truncate text-text">{r.from_name}</span>
                        <span className="truncate text-text-dim">
                          {name}
                          {parent && ` · ${parent}`}
                        </span>
                      </span>
                    </button>
                  );
                })}
              </NeighborSection>
            ))}

            {/* ── Same-file symbols (deep inspector) ──────────────────── */}
            {sameFile && sameFile.length > 0 && (
              <NeighborSection icon={Layers} label="Same file" count={sameFile.length}>
                {sameFile.slice(0, 40).map((s) => {
                  const { name, parent } = fileName(s.file_path);
                  return (
                    <button
                      key={s.id}
                      type="button"
                      title={`${s.name} · ${s.file_path}`}
                      onClick={() => onOpenSymbol?.(s)}
                      className="group flex w-full items-center gap-2 px-3 py-1.5 text-left font-mono text-mono-mini text-text-muted hover:bg-surface-overlay hover:text-text cursor-pointer transition-colors duration-150 ease-out"
                    >
                      <Layers className="h-3 w-3 shrink-0 text-text-dim group-hover:text-accent" strokeWidth={2} />
                      <span className="flex min-w-0 flex-1 flex-col">
                        <span className="truncate text-text">{s.name}</span>
                        <span className="truncate text-text-dim">
                          {s.module_path || parent || name}
                        </span>
                      </span>
                      <span className="shrink-0 font-mono text-mono-micro uppercase tracking-[0.06em] text-text-dim">
                        {s.kind}
                      </span>
                    </button>
                  );
                })}
              </NeighborSection>
            )}

            {/* ── Mentions (semantic) ─────────────────────────────────── */}
            {mentions && mentions.length > 0 && (
              <NeighborSection icon={Quote} label="Mentions" count={mentions.length}>
                {mentions.map((r, i) => {
                  const id = r.content_id.replace(/\\/g, "/");
                  const tail = id.split("/").slice(-2).join("/");
                  return (
                    <button
                      key={`${r.content_id}:${i}`}
                      type="button"
                      title={r.heading_path.join(" / ") || tail}
                      onClick={() => onOpenSection?.(r)}
                      className="group flex w-full items-center gap-2 px-3 py-1.5 text-left font-mono text-mono-mini text-text-muted hover:bg-surface-overlay hover:text-text cursor-pointer transition-colors duration-150 ease-out"
                    >
                      <Quote className="h-3 w-3 shrink-0 text-text-dim group-hover:text-accent" strokeWidth={2} />
                      <span className="flex min-w-0 flex-1 flex-col">
                        <span className="truncate text-text">{tail}</span>
                        {r.heading_path.length > 0 && (
                          <span className="truncate text-text-dim">
                            {r.heading_path.join(" / ")}
                          </span>
                        )}
                      </span>
                      <span className="shrink-0 tabular-nums text-text-dim">
                        {Math.round(r.score * 100)}%
                      </span>
                    </button>
                  );
                })}
              </NeighborSection>
            )}

            {/* ── Empty neighborhood ──────────────────────────────────── */}
            {!loading &&
              totalEdges === 0 &&
              !(sameFile && sameFile.length > 0) &&
              !(mentions && mentions.length > 0) && (
                <p className="px-3 py-4 font-mono text-mono-mini text-text-dim">
                  No neighbors in the code graph yet — nothing references{" "}
                  <span className="text-text">{symbolName}</span>.
                </p>
              )}
          </>
        )}
      </div>
    </div>
  );
}

function GlanceChip({
  label,
  value,
  accent = false,
}: {
  label: string;
  value: number;
  accent?: boolean;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 font-mono text-mono-micro uppercase tracking-[0.06em]",
        accent
          ? "border-accent/40 bg-surface text-accent"
          : "border-border-soft bg-surface-overlay text-text-dim",
      )}
    >
      <span className="tabular-nums font-semibold">{value}</span>
      {label}
    </span>
  );
}

function NeighborSection({
  icon: Icon,
  label,
  count,
  accent = false,
  children,
}: {
  icon: typeof GitFork;
  label: string;
  count: number;
  accent?: boolean;
  children: React.ReactNode;
}) {
  return (
    <section className="border-b border-border-soft last:border-b-0">
      <header className="flex items-center gap-2 px-3 pb-1 pt-2.5">
        <Icon className={cn("h-3 w-3", accent ? "text-accent" : "text-text-dim")} strokeWidth={2.25} />
        <span className="font-mono text-mono-micro font-bold uppercase tracking-[0.08em] text-text-dim">
          {label}
        </span>
        <span className="ml-auto font-mono text-mono-micro tabular-nums text-text-dim">
          {count}
        </span>
      </header>
      <div className="pb-1">{children}</div>
    </section>
  );
}

// ───────────────────────────────────────────────────────────────────────────
// Connector — fetches the symbol's definition + incoming edges in parallel.

interface ConnectorProps {
  corpusId: string;
  symbolId: string;
  symbolName: string;
  onGoToDefinition: (filePath: string, line: number) => void;
  onJumpRef: (ref: SymbolRef) => void;
  onClose: () => void;
}

export function SymbolNeighborhoodConnector({
  corpusId,
  symbolId,
  symbolName,
  onGoToDefinition,
  onJumpRef,
  onClose,
}: ConnectorProps) {
  const [definition, setDefinition] = useState<SymbolDefinitionDetail | null>(null);
  const [references, setReferences] = useState<SymbolRef[]>([]);
  const [loading, setLoading] = useState(true);
  const { openEntity } = useEntityPanel();
  const workspace = useWorkspaceOptional();

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setDefinition(null);
    setReferences([]);

    Promise.all([
      invoke<SymbolDefinitionDetail>("symbol_definition", { corpusId, symbolId }).catch(
        () => null,
      ),
      invoke<SymbolRef[]>("symbol_references", { corpusId, symbolId }).catch(
        () => [] as SymbolRef[],
      ),
    ]).then(([def, refs]) => {
      if (cancelled) return;
      setDefinition(def);
      setReferences(refs);
      setLoading(false);
    });

    return () => {
      cancelled = true;
    };
  }, [corpusId, symbolId]);

  return (
    <SymbolNeighborhood
      symbolName={symbolName}
      definition={definition}
      references={references}
      loading={loading}
      onClose={onClose}
      onGoToDefinition={onGoToDefinition}
      onJumpRef={onJumpRef}
      onInspect={
        definition
          ? () =>
              openEntity({
                kind: "symbol",
                corpusId,
                symbol: {
                  id: definition.id,
                  name: definition.name,
                  kind: definition.kind,
                  file_path: definition.file_path,
                  visibility: definition.visibility,
                  signature: definition.signature,
                  doc_comment: definition.doc_comment,
                  module_path: definition.heading_path?.join("::") ?? "",
                },
              })
          : undefined
      }
      onAsk={
        definition && workspace
          ? () => workspace.askAbout(definition.id)
          : undefined
      }
    />
  );
}
