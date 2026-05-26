import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowLeft, ArrowRight, ExternalLink, X } from "lucide-react";
import { Button } from "./ui/button";
import { cn } from "../lib/utils";
import { useEntityPanel } from "../hooks/useEntityPanel";
import type {
  DaemonStatus,
  SymbolDefinitionDetail,
  SymbolInfo,
  SymbolRef,
} from "../lib/types";

interface Props {
  status: DaemonStatus;
  activeCorpusId: string | null;
  setActiveCorpusId: (id: string | null) => void;
}

interface GraphNode {
  id: string;
  name: string;
  kind: string;
  x: number;
  y: number;
  isCenter: boolean;
  isFromKind?: string;
}

interface GraphEdge {
  from: string;
  to: string;
  kind: string;
}

const KIND_FILTERS = [
  "fn",
  "struct",
  "trait",
  "enum",
  "impl",
  "type",
  "const",
  "module",
] as const;

const REF_KIND_LABELS: Record<string, string> = {
  calls: "CALLS",
  imports: "IMPORTS",
  implements: "IMPL",
  uses: "USES",
};

const TRAIL_LIMIT = 8;

export function SymbolGraph({ status, activeCorpusId }: Props) {
  const { openEntity } = useEntityPanel();
  const corpusId = activeCorpusId ?? status.corpora[0]?.id ?? "";
  const [query, setQuery] = useState("");
  const [activeKinds, setActiveKinds] = useState<Set<string>>(new Set());
  const [activeRefKinds, setActiveRefKinds] = useState<Set<string>>(new Set());
  const [symbols, setSymbols] = useState<SymbolInfo[]>([]);
  const [selected, setSelected] = useState<SymbolInfo | null>(null);
  const [definition, setDefinition] = useState<SymbolDefinitionDetail | null>(
    null,
  );
  const [refs, setRefs] = useState<SymbolRef[]>([]);
  const [loading, setLoading] = useState(false);
  const [trail, setTrail] = useState<SymbolInfo[]>([]);
  const skipTrailRef = useRef(false);

  // Reset state when active corpus switches.
  useEffect(() => {
    setSymbols([]);
    setSelected(null);
    setDefinition(null);
    setRefs([]);
    setQuery("");
    setTrail([]);
  }, [corpusId]);

  // Cold-load: pre-populate the symbol list with top symbols (empty query).
  useEffect(() => {
    if (!corpusId) return;
    let cancelled = false;
    setLoading(true);
    invoke<SymbolInfo[]>("search_symbols", {
      corpusId,
      query: "",
      kind: null,
      filePath: null,
    })
      .then((r) => {
        if (cancelled) return;
        // Cap initial cold-load to 50 to keep the list scannable.
        setSymbols(r.slice(0, 50));
      })
      .catch(() => {})
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [corpusId]);

  async function searchSymbols() {
    if (!corpusId) return;
    setLoading(true);
    try {
      const r = await invoke<SymbolInfo[]>("search_symbols", {
        corpusId,
        query: query.trim(),
        kind: null,
        filePath: null,
      });
      setSymbols(r);
    } catch {
      /* ignore */
    } finally {
      setLoading(false);
    }
  }

  const selectSymbol = useCallback(
    async (sym: SymbolInfo) => {
      // Push the previously-selected symbol into the trail before switching.
      if (selected && !skipTrailRef.current && selected.id !== sym.id) {
        setTrail((prev) => {
          const filtered = prev.filter((t) => t.id !== selected.id);
          return [...filtered, selected].slice(-TRAIL_LIMIT);
        });
      }
      skipTrailRef.current = false;
      setSelected(sym);
      setDefinition(null);
      setRefs([]);
      setActiveRefKinds(new Set());
      try {
        const [def, references] = await Promise.all([
          invoke<SymbolDefinitionDetail>("symbol_definition", {
            corpusId,
            symbolId: sym.id,
          }).catch(() => null),
          invoke<SymbolRef[]>("symbol_references", {
            corpusId,
            symbolId: sym.id,
          }).catch(() => [] as SymbolRef[]),
        ]);
        if (def) setDefinition(def);
        setRefs(references);
      } catch {
        /* ignore */
      }
    },
    [corpusId, selected],
  );

  // Try to resolve a graph node click to a SymbolInfo (already in our list)
  // and pivot. Falls back to a one-shot search by name if not found.
  async function pivotToName(name: string) {
    const existing = symbols.find((s) => s.name === name);
    if (existing) {
      selectSymbol(existing);
      return;
    }
    try {
      const r = await invoke<SymbolInfo[]>("search_symbols", {
        corpusId,
        query: name,
        kind: null,
        filePath: null,
      });
      const exact = r.find((s) => s.name === name) ?? r[0];
      if (exact) {
        // Optionally fold into the visible list so the user sees where they went.
        setSymbols((prev) => {
          if (prev.some((p) => p.id === exact.id)) return prev;
          return [exact, ...prev].slice(0, 200);
        });
        selectSymbol(exact);
      }
    } catch {
      /* ignore */
    }
  }

  function jumpToTrailEntry(idx: number) {
    const sym = trail[idx];
    if (!sym) return;
    // Truncate trail past the clicked entry; do not re-push current.
    setTrail((prev) => prev.slice(0, idx));
    skipTrailRef.current = true;
    selectSymbol(sym);
  }

  function clearTrail() {
    setTrail([]);
  }

  function toggleKind(kind: string) {
    setActiveKinds((prev) => {
      const next = new Set(prev);
      if (next.has(kind)) next.delete(kind);
      else next.add(kind);
      return next;
    });
  }

  function toggleRefKind(kind: string) {
    setActiveRefKinds((prev) => {
      const next = new Set(prev);
      if (next.has(kind)) next.delete(kind);
      else next.add(kind);
      return next;
    });
  }

  const visibleSymbols = useMemo(() => {
    if (activeKinds.size === 0) return symbols;
    return symbols.filter((s) => activeKinds.has(s.kind));
  }, [symbols, activeKinds]);

  const refKindCounts = useMemo(() => {
    const m = new Map<string, number>();
    for (const r of refs) m.set(r.ref_kind, (m.get(r.ref_kind) ?? 0) + 1);
    return m;
  }, [refs]);

  const visibleRefs = useMemo(() => {
    if (activeRefKinds.size === 0) return refs;
    return refs.filter((r) => activeRefKinds.has(r.ref_kind));
  }, [refs, activeRefKinds]);

  const { nodes, edges } = useMemo(() => {
    const nodes: GraphNode[] = [];
    const edges: GraphEdge[] = [];
    if (!selected) return { nodes, edges };

    const nodeSet = new Map<string, GraphNode>();
    nodeSet.set(selected.name, {
      id: selected.id,
      name: selected.name,
      kind: selected.kind,
      x: 0,
      y: 0,
      isCenter: true,
    });

    for (const r of visibleRefs) {
      if (!nodeSet.has(r.from_name)) {
        nodeSet.set(r.from_name, {
          id: r.from_name,
          name: r.from_name,
          kind: "ref",
          x: 0,
          y: 0,
          isCenter: false,
          isFromKind: r.ref_kind,
        });
      }
      if (!nodeSet.has(r.to_name)) {
        nodeSet.set(r.to_name, {
          id: r.to_name,
          name: r.to_name,
          kind: "ref",
          x: 0,
          y: 0,
          isCenter: false,
        });
      }
      edges.push({ from: r.from_name, to: r.to_name, kind: r.ref_kind });
    }

    const all = Array.from(nodeSet.values());
    const cx = 250;
    const cy = 175;
    const radius = Math.min(140, Math.max(60, all.length * 18));
    let i = 0;
    all.forEach((n) => {
      if (n.isCenter) {
        n.x = cx;
        n.y = cy;
      } else {
        const angle =
          (2 * Math.PI * i) / Math.max(all.length - 1, 1);
        n.x = cx + radius * Math.cos(angle);
        n.y = cy + radius * Math.sin(angle);
        i++;
      }
    });
    nodes.push(...all);
    return { nodes, edges };
  }, [selected, visibleRefs]);

  return (
    <div className="@container/page flex flex-col gap-3 h-full min-h-0">
      {/* Trail strip — visible only with 1+ entries */}
      {trail.length > 0 && (
        <div className="flex items-center gap-2 rounded-lg border border-border-soft bg-surface-overlay px-2 py-1 shrink-0">
          <span className="font-sans text-sm font-bold text-text-dim shrink-0">
            Trail
          </span>
          <div className="flex items-center gap-1 flex-wrap min-w-0">
            {trail.map((t, i) => (
              <button
                key={`${t.id}-${i}`}
                onClick={() => jumpToTrailEntry(i)}
                className="inline-flex items-center gap-1 border border-border-soft bg-surface px-2 py-0.5 font-mono text-xs font-semibold text-text-muted hover:text-text hover:border-border cursor-pointer transition-colors duration-150 ease-out rounded-md"
              >
                <ArrowLeft className="h-2.5 w-2.5" strokeWidth={2} />
                <span className="truncate max-w-[120px]">{t.name}</span>
              </button>
            ))}
          </div>
          <button
            onClick={clearTrail}
            aria-label="Clear trail"
            className="ml-auto grid h-5 w-5 place-items-center border border-border-soft text-text-muted hover:text-text hover:border-border cursor-pointer transition-colors duration-150 ease-out shrink-0 rounded-md"
          >
            <X className="h-2.5 w-2.5" strokeWidth={2} />
          </button>
        </div>
      )}

      {/* Three-pane layout: filters+list / graph / detail */}
      <div className="flex-1 min-h-0 grid grid-cols-1 @min-[820px]/page:grid-cols-[clamp(180px,22%,280px)_minmax(0,1fr)_clamp(280px,28%,380px)] gap-3">
        {/* LEFT: filters + symbol list */}
        <div className="flex flex-col gap-2 min-h-0">
          <form
            onSubmit={(e) => {
              e.preventDefault();
              searchSymbols();
            }}
            className="flex gap-2"
          >
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="search symbols"
              className="h-9 flex-1 rounded-md border border-border-soft bg-surface px-2 text-sm font-sans text-text placeholder:text-text-dim focus:outline-none focus:border-accent transition-colors duration-150 ease-out"
            />
            <Button type="submit" disabled={loading} size="default">
              {loading ? "…" : "Go"}
            </Button>
          </form>

          <div className="flex flex-wrap gap-1">
            {KIND_FILTERS.map((k) => {
              const active = activeKinds.has(k);
              return (
                <button
                  key={k}
                  onClick={() => toggleKind(k)}
                  className={cn(
                    "border px-2 py-0.5 text-mono-mini font-mono font-semibold uppercase tracking-[0.08em] cursor-pointer transition-colors duration-150 ease-out",
                    active
                      ? "border-accent bg-surface-overlay text-text"
                      : "border-border-soft bg-surface text-text-muted hover:border-border hover:text-text",
                  "rounded-md")}
                >
                  {k}
                </button>
              );
            })}
          </div>

          <div className="flex-1 min-h-0 overflow-y-auto rounded-lg border border-border-soft bg-surface overflow-hidden">
            <div className="flex items-baseline justify-between gap-3 border-b border-border-soft bg-surface-overlay px-3 py-1.5 sticky top-0">
              <h3 className="font-sans text-sm font-semibold text-text">
                {symbols.length === 0 && !query ? "Browse" : "Matches"}
              </h3>
              <span className="font-mono text-xs tabular-nums text-text-dim">
                {visibleSymbols.length}
              </span>
            </div>
            {visibleSymbols.length === 0 ? (
              <p className="px-3 py-4 font-sans text-sm text-text-dim">
                {loading ? "Loading_" : query ? "No matches." : "No symbols."}
              </p>
            ) : (
              visibleSymbols.map((s) => {
                const isSelected = selected?.id === s.id;
                return (
                  <button
                    key={s.id}
                    onClick={() => selectSymbol(s)}
                    className={cn(
                      "relative w-full text-left flex items-center gap-2 px-3 py-1.5 border-b border-border-soft last:border-b-0 cursor-pointer transition-colors duration-150 ease-out",
                      isSelected
                        ? "bg-surface-overlay text-text"
                        : "text-text-muted hover:bg-surface-overlay hover:text-text",
                    )}
                  >
                    {isSelected && (
                      <span className="absolute left-0 top-0 bottom-0 w-[3px] bg-accent" />
                    )}
                    <span className="font-mono text-mono-mini uppercase tracking-[0.08em] w-12 shrink-0 text-text-dim">
                      {s.kind}
                    </span>
                    <span className="font-mono text-sm font-semibold truncate">
                      {s.name}
                    </span>
                  </button>
                );
              })
            )}
          </div>
        </div>

        {/* CENTER: graph (clickable nodes). min-w-0 so the SVG doesn't push the grid. */}
        <div className="rounded-lg border border-border-soft bg-surface min-h-0 min-w-0 overflow-hidden">
          {selected ? (
            <svg viewBox="0 0 500 350" className="w-full h-full">
              {edges.map((e, i) => {
                const from = nodes.find((n) => n.name === e.from);
                const to = nodes.find((n) => n.name === e.to);
                if (!from || !to) return null;
                return (
                  <line
                    key={i}
                    x1={from.x}
                    y1={from.y}
                    x2={to.x}
                    y2={to.y}
                    stroke="var(--color-border)"
                    strokeWidth={2}
                  />
                );
              })}

              {nodes.map((n) => {
                const half = n.isCenter ? 12 : 7;
                return (
                  <g
                    key={n.name}
                    onClick={() => {
                      if (!n.isCenter) pivotToName(n.name);
                    }}
                    style={{
                      cursor: n.isCenter ? "default" : "pointer",
                    }}
                  >
                    <rect
                      x={n.x - half}
                      y={n.y - half}
                      width={half * 2}
                      height={half * 2}
                      fill={
                        n.isCenter
                          ? "var(--color-accent)"
                          : "var(--color-surface)"
                      }
                      stroke="var(--color-border)"
                      strokeWidth={2}
                    />
                    <text
                      x={n.x}
                      y={n.y + (n.isCenter ? 26 : 22)}
                      textAnchor="middle"
                      fill="var(--color-text)"
                      fontSize={n.isCenter ? "0.6875rem" : "0.625rem"}
                      fontFamily="var(--font-mono)"
                      fontWeight={n.isCenter ? 700 : 500}
                      style={{ pointerEvents: "none" }}
                    >
                      {n.name.length > 22 ? n.name.slice(0, 22) + "…" : n.name}
                    </text>
                  </g>
                );
              })}
            </svg>
          ) : (
            <div className="flex flex-col items-center justify-center gap-2 h-full text-center">
              <div className="grid h-12 w-12 place-items-center rounded-md border border-border-soft bg-surface-overlay text-text-muted">
                ⌺
              </div>
              <p className="font-sans text-xs font-semibold tracking-[0.08em] text-text">
                Pick a symbol
              </p>
              <p className="max-w-xs font-sans text-xs tracking-[0.08em] text-text-dim">
                Click any node in the graph to pivot.
              </p>
            </div>
          )}
        </div>

        {/* RIGHT: definition + references */}
        <div className="flex flex-col gap-3 min-h-0 min-w-0 overflow-y-auto">
          {selected ? (
            <>
              <section className="rounded-lg border border-border-soft bg-surface p-3 space-y-2">
                <div className="flex items-center justify-between gap-2">
                  <span className="font-mono text-xs font-bold uppercase tracking-[0.08em] text-accent">
                    Definition
                  </span>
                  <div className="flex items-center gap-2">
                    {selected.visibility && (
                      <span className="font-mono text-xs uppercase tracking-[0.08em] text-text-dim">
                        {selected.visibility}
                      </span>
                    )}
                    <button
                      onClick={() =>
                        openEntity({
                          kind: "symbol",
                          corpusId,
                          symbol: selected,
                        })
                      }
                      className="inline-flex items-center gap-1 rounded-md border border-border bg-surface px-1.5 py-0.5 font-mono text-xs font-bold uppercase tracking-[0.08em] text-text hover:bg-surface-overlay hover:text-text cursor-pointer transition-colors duration-150 ease-out"
                      title="Open full panel"
                    >
                      <ExternalLink className="h-3 w-3" strokeWidth={2.5} />
                      Full panel
                    </button>
                  </div>
                </div>
                {definition ? (
                  <>
                    <div className="font-mono text-xs font-bold text-text">
                      {definition.signature}
                    </div>
                    <div className="font-mono text-xs text-text-dim">
                      {definition.file_path}:{definition.line_start}-
                      {definition.line_end}
                    </div>
                    {definition.doc_comment && (
                      <div className="border-l-2 border-accent bg-surface-overlay px-2 py-1.5 font-mono text-mono-mini text-text-muted whitespace-pre-wrap">
                        {definition.doc_comment}
                      </div>
                    )}
                    <pre className="rounded-md border border-border-soft bg-surface-sunken p-2 font-mono text-mono-mini leading-relaxed text-text whitespace-pre overflow-x-auto max-h-72 overflow-y-auto">
                      <JumpableSource
                        source={definition.source_context}
                        symbols={symbols}
                        currentName={selected.name}
                        onPivot={pivotToName}
                      />
                    </pre>
                  </>
                ) : (
                  <p className="font-mono text-mono-mini text-text-dim">
                    {selected.signature}
                  </p>
                )}
              </section>

              <section className="rounded-lg border border-border-soft bg-surface overflow-hidden">
                <div className="border-b border-border-soft bg-surface-overlay px-3 py-1.5 flex items-center justify-between">
                  <span className="font-sans text-xs font-semibold text-text">
                    References
                  </span>
                  <span className="font-mono text-xs tabular-nums text-text-dim">
                    {visibleRefs.length}
                    {activeRefKinds.size > 0 && ` / ${refs.length}`}
                  </span>
                </div>

                {/* Ref-kind filter pills */}
                {refs.length > 0 && (
                  <div className="flex flex-wrap gap-1 px-2 py-1 border-b border-border">
                    {Array.from(refKindCounts.entries()).map(([k, c]) => {
                      const active = activeRefKinds.has(k);
                      return (
                        <button
                          key={k}
                          onClick={() => toggleRefKind(k)}
                          className={cn(
                            "inline-flex items-center gap-1 border border-border px-1.5 py-0.5 font-mono text-xs font-bold uppercase tracking-[0.08em] cursor-pointer transition-colors duration-150 ease-out",
                            active
                              ? "bg-accent text-[var(--color-accent-fg-on)]"
                              : "bg-surface text-text hover:bg-surface-overlay",
                          )}
                        >
                          <span>{REF_KIND_LABELS[k] ?? k.toUpperCase()}</span>
                          <span className="opacity-70 tabular-nums">{c}</span>
                        </button>
                      );
                    })}
                  </div>
                )}

                {visibleRefs.length === 0 ? (
                  <p className="px-2 py-3 font-mono text-mono-mini tracking-[0.08em] text-text-dim">
                    {refs.length === 0 ? "no references" : "no matches"}
                  </p>
                ) : (
                  visibleRefs.map((r, i) => (
                    <button
                      key={i}
                      onClick={() => pivotToName(r.from_name)}
                      title={`Pivot to ${r.from_name}`}
                      className="w-full text-left flex items-center gap-2 border-b border-border last:border-b-0 px-2 py-1.5 font-mono text-mono-mini cursor-pointer transition-colors duration-150 ease-out hover:bg-surface-overlay hover:text-text"
                    >
                      <span className="text-text truncate flex-1">
                        {r.from_name}
                      </span>
                      <ArrowRight
                        className="h-3 w-3 shrink-0"
                        strokeWidth={2.5}
                      />
                      <span className="text-text truncate flex-1">
                        {r.to_name}
                      </span>
                      <span className="border border-border-soft px-1 text-mono-micro uppercase tracking-[0.08em] opacity-70 shrink-0">
                        {r.ref_kind}
                      </span>
                    </button>
                  ))
                )}
              </section>
            </>
          ) : (
            <div className="border border-dotted border-border bg-surface px-3 py-6 text-center font-sans text-xs tracking-[0.08em] text-text-dim">
              Select a symbol for details
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

// ─── JUMPABLE SOURCE ──────────────────────────────────────────────────────

const RUST_KEYWORDS = new Set([
  "fn", "pub", "struct", "trait", "enum", "impl", "type", "const", "let",
  "mut", "use", "mod", "match", "if", "else", "for", "while", "loop", "return",
  "self", "Self", "where", "as", "in", "ref", "move", "async", "await", "dyn",
  "true", "false", "None", "Some", "Ok", "Err", "Box", "Vec", "String", "Option",
  "Result", "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "f32", "f64",
  "usize", "isize", "bool", "char", "str", "static", "extern", "unsafe", "crate",
]);

function tokenizeSource(source: string): { type: "ident" | "text"; value: string }[] {
  const out: { type: "ident" | "text"; value: string }[] = [];
  // Match identifiers (letters/underscore start) anywhere; everything else
  // is plain text (whitespace, punctuation, numbers, strings — left as-is).
  const re = /([A-Za-z_][A-Za-z0-9_]*)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(source)) !== null) {
    if (m.index > last) {
      out.push({ type: "text", value: source.slice(last, m.index) });
    }
    out.push({ type: "ident", value: m[1] });
    last = m.index + m[1].length;
  }
  if (last < source.length) out.push({ type: "text", value: source.slice(last) });
  return out;
}

function JumpableSource({
  source,
  symbols,
  currentName,
  onPivot,
}: {
  source: string;
  symbols: SymbolInfo[];
  currentName: string;
  onPivot: (name: string) => void;
}) {
  const known = useMemo(() => {
    const m = new Set<string>();
    for (const s of symbols) m.add(s.name);
    return m;
  }, [symbols]);

  const tokens = useMemo(() => tokenizeSource(source), [source]);

  return (
    <>
      {tokens.map((t, i) => {
        if (t.type === "text") return <span key={i}>{t.value}</span>;
        // Skip the symbol's own name (no point pivoting to self), keywords,
        // and anything that doesn't match a known indexed symbol.
        const jumpable =
          t.value !== currentName &&
          !RUST_KEYWORDS.has(t.value) &&
          known.has(t.value);
        if (!jumpable) return <span key={i}>{t.value}</span>;
        return (
          <span
            key={i}
            role="button"
            tabIndex={0}
            onClick={(e) => {
              e.stopPropagation();
              onPivot(t.value);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                e.stopPropagation();
                onPivot(t.value);
              }
            }}
            className="underline decoration-2 decoration-accent underline-offset-2 cursor-pointer hover:bg-surface-overlay hover:text-text focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent rounded-sm"
            title={`Pivot to ${t.value}`}
          >
            {t.value}
          </span>
        );
      })}
    </>
  );
}
