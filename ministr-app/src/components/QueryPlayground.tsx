import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, ChevronRight, RefreshCw, X } from "lucide-react";
import { Button } from "./ui/button";
import { EmptyState } from "./ui/empty-state";
import { FilterPill } from "./ui/filter-pill";
import { cn } from "../lib/utils";
import { relative } from "../lib/time";
import { corpusRelative } from "../lib/path";
import { useEntityPanel } from "../hooks/useEntityPanel";
import type {
  BridgeLink,
  CoherenceEvent,
  CoherenceKind,
  CorpusInfo,
  DaemonStatus,
  FileInfo,
  SearchResult,
  SymbolInfo,
} from "../lib/types";

interface Props {
  status: DaemonStatus;
  activeCorpusId: string | null;
  setActiveCorpusId: (id: string | null) => void;
}

type KindFilter = "all" | "sections" | "symbols" | "bridges";

const FALLBACK_PROBES = [
  "authentication",
  "Config",
  "main",
  "error",
  "startup",
  "Storage",
  "tauri_command",
  "test",
];

const HISTORY_LIMIT = 10;

export function QueryPlayground({ status, activeCorpusId }: Props) {
  const { openEntity } = useEntityPanel();
  const [query, setQuery] = useState("");
  const corpusId = activeCorpusId ?? status.corpora[0]?.id ?? "";
  const [results, setResults] = useState<SearchResult[]>([]);
  const [symbols, setSymbols] = useState<SymbolInfo[]>([]);
  const [bridges, setBridges] = useState<BridgeLink[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [compact, setCompact] = useState(false);
  const [browse, setBrowse] = useState(false);
  const [history, setHistory] = useState<string[]>([]);
  const [inputFocused, setInputFocused] = useState(false);
  const [hotFiles, setHotFiles] = useState<FileInfo[] | null>(null);
  const [allSymbols, setAllSymbols] = useState<SymbolInfo[] | null>(null);
  const [kindFilter, setKindFilter] = useState<KindFilter>("all");
  const lastQueryRef = useRef<string | null>(null);

  const selectedCorpus = useMemo(
    () => status.corpora.find((c) => c.id === corpusId) ?? null,
    [status.corpora, corpusId],
  );

  // Clear results when active corpus switches at the shell level.
  useEffect(() => {
    clearResults();
    setQuery("");
    setError(null);
    setHistory([]);
    setHotFiles(null);
    setAllSymbols(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [corpusId]);

  // (Esc-to-close lives in the EntityPanel's own provider now.)

  // Fetch dynamic probe sources whenever the active corpus changes.
  useEffect(() => {
    if (!corpusId) return;
    let cancelled = false;
    invoke<FileInfo[]>("list_corpus_files", { corpusId })
      .then((files) => {
        if (!cancelled) setHotFiles(files);
      })
      .catch(() => {});
    invoke<SymbolInfo[]>("search_symbols", {
      corpusId,
      query: "",
      kind: null,
      filePath: null,
    })
      .then((syms) => {
        if (!cancelled) setAllSymbols(syms);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [corpusId]);

  // Probes derived from corpus content. Now corpus-aware (no mode).
  const probes = useMemo<string[]>(() => {
    const out: string[] = [];
    if (allSymbols && allSymbols.length > 0) {
      const byKind = new Map<string, number>();
      for (const s of allSymbols)
        byKind.set(s.kind, (byKind.get(s.kind) ?? 0) + 1);
      const kinds = Array.from(byKind.entries())
        .sort((a, b) => b[1] - a[1])
        .slice(0, 2)
        .map(([k]) => k);
      const names = allSymbols.slice(0, 3).map((s) => s.name);
      out.push(...kinds, ...names);
    }
    if (hotFiles && hotFiles.length > 0) {
      const top = [...hotFiles]
        .sort((a, b) => b.section_count - a.section_count)
        .slice(0, 3)
        .map((f) => {
          const base = f.path.split(/[\\/]/).pop() ?? f.path;
          return base.replace(/\.[^.]+$/, "");
        });
      out.push(...top);
    }
    const merged = Array.from(new Set(out)).slice(0, 8);
    return merged.length >= 4 ? merged : FALLBACK_PROBES;
  }, [allSymbols, hotFiles]);

  function pushHistory(q: string) {
    const v = q.trim();
    if (!v) return;
    setHistory((prev) => {
      const next = [v, ...prev.filter((x) => x !== v)];
      return next.slice(0, HISTORY_LIMIT);
    });
  }

  function clearResults() {
    setResults([]);
    setSymbols([]);
    setBridges([]);
  }

  /**
   * Run the unified search: fan out three queries in parallel and merge.
   * No "mode" anymore — typing anywhere produces a blended result list
   * grouped by kind. Spotlight-style.
   */
  async function submit(currentQuery = query) {
    if (!corpusId) return;
    const q = currentQuery.trim();
    if (!q) return;
    setLoading(true);
    setError(null);
    setBrowse(false);
    lastQueryRef.current = q;

    const [sectionsR, symbolsR, bridgesR] = await Promise.allSettled([
      invoke<SearchResult[]>("search_corpus", {
        corpusId,
        query: q,
        topK: 20,
      }),
      invoke<SymbolInfo[]>("search_symbols", {
        corpusId,
        query: q,
        kind: null,
        filePath: null,
      }),
      invoke<BridgeLink[]>("bridge_query", {
        corpusId,
        query: q,
        kind: null,
        sourceLanguage: null,
        filePath: null,
        limit: 200,
      }),
    ]);

    setResults(sectionsR.status === "fulfilled" ? sectionsR.value : []);
    setSymbols(symbolsR.status === "fulfilled" ? symbolsR.value : []);
    setBridges(bridgesR.status === "fulfilled" ? bridgesR.value : []);

    const failures = [sectionsR, symbolsR, bridgesR].filter(
      (r) => r.status === "rejected",
    ) as PromiseRejectedResult[];
    if (failures.length === 3) {
      setError(String(failures[0].reason ?? "all queries failed"));
    }

    pushHistory(q);
    setKindFilter("all");
    setLoading(false);
  }

  function retry() {
    const last = lastQueryRef.current;
    if (!last) return;
    submit(last);
  }

  /**
   * "Filtered" runs — used by landing-tile click-to-drill-in. Still single
   * fan-out searches, just with one of the kind-specific filters set.
   */
  async function runSymbolsByFile(filePath: string) {
    if (!corpusId) return;
    setLoading(true);
    setError(null);
    setBrowse(false);
    try {
      const r = await invoke<SymbolInfo[]>("search_symbols", {
        corpusId,
        query: "",
        kind: null,
        filePath,
      });
      setSymbols(r);
      setResults([]);
      setBridges([]);
      setKindFilter("symbols");
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  async function runBridgeByKind(kind: string) {
    if (!corpusId) return;
    setLoading(true);
    setError(null);
    setBrowse(false);
    try {
      const r = await invoke<BridgeLink[]>("bridge_query", {
        corpusId,
        query: null,
        kind,
        sourceLanguage: null,
        filePath: null,
        limit: 200,
      });
      setBridges(r);
      setResults([]);
      setSymbols([]);
      setKindFilter("bridges");
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  function openSymbolDetail(symbol: SymbolInfo) {
    openEntity({ kind: "symbol", corpusId, symbol });
  }

  function openSectionDetail(r: SearchResult) {
    openEntity({ kind: "section", corpusId, result: r });
  }

  function openBridgeDetail(b: BridgeLink) {
    openEntity({ kind: "bridge", corpusId, link: b });
  }

  const hasResults =
    results.length > 0 || symbols.length > 0 || bridges.length > 0;
  const showLanding =
    browse || (!hasResults && !loading && !query.trim() && !error);

  function applyProbe(p: string) {
    setQuery(p);
    submit(p);
  }

  return (
    <div className="@container/page flex h-full gap-0 @max-[1439px]/page:flex-col @max-[1439px]/page:gap-0">
      {/* LEFT: search + landing/results */}
      <div className="flex-1 min-w-0 flex flex-col gap-3 min-h-0">
        {/* Omnibar — single unified entry point. No mode tabs. */}
        <form
          onSubmit={(e) => {
            e.preventDefault();
            submit();
          }}
          className="flex flex-col gap-2"
        >
          <div className="flex items-center gap-2">
            <span className="font-mono text-lg font-bold text-accent">{">"}</span>
            <input
              value={query}
              onFocus={() => setInputFocused(true)}
              onBlur={() => setTimeout(() => setInputFocused(false), 100)}
              onChange={(e) => {
                setQuery(e.target.value);
                if (e.target.value === "") clearResults();
              }}
              placeholder="search anything · sections, symbols, bridges"
              className="h-12 flex-1 border border-border-soft bg-surface px-3 text-base font-sans text-text placeholder:text-text-dim placeholder:normal-case focus:outline-none focus:border-accent transition-colors duration-150 ease-out"
            />
            <Button type="submit" size="lg" disabled={loading}>
              {loading ? "…" : "Run"}
            </Button>
            <ViewToggle
              label="Compact"
              active={compact}
              onClick={() => setCompact((v) => !v)}
            />
            <ViewToggle
              label="Browse"
              active={browse}
              onClick={() => setBrowse((v) => !v)}
              disabled={!hasResults}
            />
          </div>
        </form>

        {/* History pills — visible only on focus and only when input is empty. */}
        {inputFocused && !query.trim() && history.length > 0 && (
          <div className="flex items-center gap-2 flex-wrap">
            <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim shrink-0">
              Recent
            </span>
            {history.map((h, i) => (
              <button
                key={`${h}-${i}`}
                onClick={() => {
                  setQuery(h);
                  submit(h);
                }}
                className="inline-flex items-center gap-1.5 border border-border-soft bg-surface px-2 py-0.5 font-sans text-sm text-text-muted hover:text-text hover:border-border cursor-pointer transition-colors duration-150 ease-out rounded-md"
              >
                <span className="font-mono text-mono-mini text-text-dim tabular-nums">{i + 1}</span>
                <span className="font-mono">{h}</span>
              </button>
            ))}
          </div>
        )}

        {/* Quick probes — always visible. Click prefills + auto-runs. */}
        <div className="flex items-center gap-2 flex-wrap">
          <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim shrink-0">
            Probes
          </span>
          {probes.map((p) => (
            <button
              key={p}
              onClick={() => applyProbe(p)}
              className="border border-border-soft bg-surface px-2 py-0.5 font-mono text-sm font-medium text-text-muted hover:text-text hover:border-border cursor-pointer transition-colors duration-150 ease-out rounded-md"
            >
              {p}
            </button>
          ))}
        </div>

        {/* Error card */}
        {error && (
          <div className="border border-danger bg-surface p-3 flex items-start gap-3 border-l-2">
            <AlertTriangle className="h-4 w-4 text-danger shrink-0 mt-0.5" strokeWidth={2} />
            <div className="flex-1 min-w-0">
              <p className="font-sans text-base font-bold text-danger">
                Query failed
              </p>
              <p className="font-sans text-sm text-text-muted mt-1 break-words">
                {error}
              </p>
            </div>
            <Button variant="outline" size="sm" onClick={retry}>
              <RefreshCw className="h-3 w-3" strokeWidth={2} />
              Retry
            </Button>
          </div>
        )}

        {/* Kind filter chips — visible only when there are results. Optional
            refinement; default is ALL (everything is shown blended). */}
        {hasResults && (
          <KindFilterStrip
            filter={kindFilter}
            onChange={setKindFilter}
            counts={{
              sections: results.length,
              symbols: symbols.length,
              bridges: bridges.length,
            }}
          />
        )}

        {/* Body — landing tiles OR unified blended results */}
        <div className="flex-1 min-h-0 overflow-y-auto">
          {showLanding ? (
            selectedCorpus ? (
              <UnifiedLanding
                corpus={selectedCorpus}
                onJumpToFile={runSymbolsByFile}
                onJumpToBridgeKind={runBridgeByKind}
              />
            ) : (
              <EmptyCorpusTile />
            )
          ) : !hasResults && !loading && query ? (
            <EmptyState
              icon={X}
              title="NO RESULTS"
              hint="Try a different query, or browse via the landing tiles."
            />
          ) : (
            <BlendedResults
              query={query}
              results={results}
              symbols={symbols}
              bridges={bridges}
              compact={compact}
              filter={kindFilter}
              corpus={selectedCorpus}
              onOpenSection={openSectionDetail}
              onOpenSymbol={openSymbolDetail}
              onOpenBridge={openBridgeDetail}
            />
          )}
        </div>
      </div>

      {/* Detail surface is now the global EntityPanel — no local aside. */}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// UNIFIED LANDING — single mode-agnostic cold-load surface (Spotlight pattern).
// ─────────────────────────────────────────────────────────────────────────────

function UnifiedLanding({
  corpus,
  onJumpToFile,
  onJumpToBridgeKind,
}: {
  corpus: CorpusInfo;
  onJumpToFile: (filePath: string) => void;
  onJumpToBridgeKind: (kind: string) => void;
}) {
  const [files, setFiles] = useState<FileInfo[] | null>(null);
  const [bridges, setBridges] = useState<BridgeLink[] | null>(null);
  const [coherence, setCoherence] = useState<CoherenceEvent[] | null>(null);
  const [allSyms, setAllSyms] = useState<SymbolInfo[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    setFiles(null);
    setBridges(null);
    setCoherence(null);
    setAllSyms(null);
    Promise.allSettled([
      invoke<FileInfo[]>("list_corpus_files", { corpusId: corpus.id }),
      invoke<BridgeLink[]>("bridge_query", {
        corpusId: corpus.id,
        query: null,
        kind: null,
        sourceLanguage: null,
        filePath: null,
        limit: 500,
      }),
      invoke<CoherenceEvent[]>("recent_coherence_events", {
        limit: 10,
        sinceMs: null,
      }),
      invoke<SymbolInfo[]>("search_symbols", {
        corpusId: corpus.id,
        query: "",
        kind: null,
        filePath: null,
      }),
    ]).then(([f, b, c, s]) => {
      if (cancelled) return;
      setFiles(f.status === "fulfilled" ? f.value : []);
      setBridges(b.status === "fulfilled" ? b.value : []);
      setCoherence(c.status === "fulfilled" ? c.value : []);
      setAllSyms(s.status === "fulfilled" ? s.value : []);
    });
    return () => {
      cancelled = true;
    };
  }, [corpus.id]);

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-3">
      <SymbolKindDashboard symbols={allSyms} corpusId={corpus.id} />
      <BridgesTile bridges={bridges} onJumpToKind={onJumpToBridgeKind} />
      <HotFilesTile files={files} corpus={corpus} onJumpToFile={onJumpToFile} />
      <RecentChangesTile events={coherence} corpus={corpus} />
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// KIND FILTER STRIP — optional refinement above blended results.
// ─────────────────────────────────────────────────────────────────────────────

function KindFilterStrip({
  filter,
  onChange,
  counts,
}: {
  filter: KindFilter;
  onChange: (f: KindFilter) => void;
  counts: { sections: number; symbols: number; bridges: number };
}) {
  const total = counts.sections + counts.symbols + counts.bridges;
  const items: { key: KindFilter; label: string; count: number }[] = [
    { key: "all", label: "All", count: total },
    { key: "sections", label: "Sections", count: counts.sections },
    { key: "symbols", label: "Symbols", count: counts.symbols },
    { key: "bridges", label: "Bridges", count: counts.bridges },
  ];
  return (
    <div className="flex items-stretch gap-0">
      {items.map(({ key, label, count }) => {
        const active = filter === key;
        return (
          <button
            key={key}
            onClick={() => onChange(key)}
            disabled={key !== "all" && count === 0}
            className={cn(
              "border border-border-soft px-3 py-1.5 font-sans text-sm font-medium cursor-pointer transition-colors duration-150 ease-out -ml-[1px] first:ml-0 inline-flex items-center gap-1.5",
              active
                ? "border-accent bg-surface-overlay text-text z-10 relative"
                : "bg-surface text-text-muted hover:bg-surface-overlay hover:text-text",
              key !== "all" && count === 0 && "opacity-40 cursor-not-allowed",
            )}
          >
            <span>{label}</span>
            <span className="font-mono text-xs tabular-nums text-text-dim">{count}</span>
          </button>
        );
      })}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// BLENDED RESULTS — three grouped sections in one scroll. Spotlight-style.
// ─────────────────────────────────────────────────────────────────────────────

function BlendedResults({
  query,
  results,
  symbols,
  bridges,
  compact,
  filter,
  corpus,
  onOpenSection,
  onOpenSymbol,
  onOpenBridge,
}: {
  query: string;
  results: SearchResult[];
  symbols: SymbolInfo[];
  bridges: BridgeLink[];
  compact: boolean;
  filter: KindFilter;
  corpus: CorpusInfo | null;
  onOpenSection: (r: SearchResult) => void;
  onOpenSymbol: (s: SymbolInfo) => void;
  onOpenBridge: (b: BridgeLink) => void;
}) {
  void query;
  const showSections = filter === "all" || filter === "sections";
  const showSymbols = filter === "all" || filter === "symbols";
  const showBridges = filter === "all" || filter === "bridges";

  // Reset the §N counter at the top of each blended-render so chapters
  // start at §1 every time the user re-runs a query.
  resetBlendedGroupIndex();

  return (
    <div className="flex flex-col gap-4">
      {showSections && results.length > 0 && (
        <BlendedGroup label="Sections" count={results.length} accent="accent">
          <div className={cn("flex flex-col", compact ? "gap-0" : "gap-2")}>
            {results.map((r, i) => (
              <SurveyCard
                key={`${r.content_id}-${i}`}
                result={r}
                compact={compact}
                corpus={corpus}
                onClick={() => onOpenSection(r)}
              />
            ))}
          </div>
        </BlendedGroup>
      )}

      {showSymbols && symbols.length > 0 && (
        <BlendedGroup label="Symbols" count={symbols.length} accent="accent">
          <div className={cn("flex flex-col", compact ? "gap-0" : "gap-2")}>
            {symbols.map((s) => (
              <SymbolCard
                key={s.id}
                symbol={s}
                compact={compact}
                onClick={() => onOpenSymbol(s)}
              />
            ))}
          </div>
        </BlendedGroup>
      )}

      {showBridges && bridges.length > 0 && (
        <BlendedGroup label="Bridges" count={bridges.length} accent="accent">
          <div className="border border-border-soft bg-surface">
            {bridges.map((b, i) => (
              <button
                key={`${b.kind}-${i}`}
                onClick={() => onOpenBridge(b)}
                className="w-full text-left grid grid-cols-[1fr_auto_1fr_auto_60px] gap-2 px-3 py-2 cursor-pointer transition-colors duration-150 ease-out border-b border-border-soft last:border-b-0 hover:bg-surface-overlay hover:text-text items-center"
              >
                <span className="flex items-center gap-2 min-w-0">
                  <span className="border border-border-soft px-1 font-mono text-mono-micro uppercase tracking-[0.08em] opacity-70 shrink-0">
                    {b.export_language}
                  </span>
                  <span className="font-mono text-xs font-bold truncate">
                    {b.export_symbol || b.export_binding_key}
                  </span>
                </span>
                <span className="font-mono text-xs uppercase tracking-[0.08em] opacity-70 shrink-0">
                  {b.kind}
                </span>
                <span className="flex items-center gap-2 min-w-0">
                  <span className="border border-border-soft px-1 font-mono text-mono-micro uppercase tracking-[0.08em] opacity-70 shrink-0">
                    {b.import_language}
                  </span>
                  <span className="font-mono text-xs font-bold truncate">
                    {b.import_symbol || b.import_binding_key}
                  </span>
                </span>
                <ChevronRight className="h-3 w-3 shrink-0" strokeWidth={2.5} />
                <span className="font-mono text-xs tabular-nums text-right shrink-0">
                  {(b.confidence * 100).toFixed(0)}%
                </span>
              </button>
            ))}
          </div>
        </BlendedGroup>
      )}
    </div>
  );
}

// Index counter for §N markers across blended-result groups within a single
// QueryPlayground render. Reset by the parent rendering pass.
let _blendedGroupIndex = 0;

function BlendedGroup({
  label,
  count,
  accent,
  children,
}: {
  label: string;
  count: number;
  accent: "accent" | "muted";
  children: React.ReactNode;
}) {
  void accent;
  // Sentence-case the label since callers pass UPPERCASE legacy values.
  const sentence = /^[A-Z][A-Z\s\-—·]+$/.test(label)
    ? label.charAt(0) + label.slice(1).toLowerCase()
    : label;
  // Increment-on-render is fine here: React renders these groups in a
  // deterministic order within the same pass.
  _blendedGroupIndex += 1;
  const idx = _blendedGroupIndex;
  return (
    <section>
      <header className="flex items-baseline gap-3 border-b border-border-soft bg-surface-overlay px-3 py-2 mb-2">
        <span className="font-sans text-base font-normal text-text-dim tabular-nums shrink-0 w-6">
          §{idx}
        </span>
        <h3 className="font-sans text-base font-bold text-text flex-1 min-w-0">
          {sentence}
        </h3>
        <span className="font-mono text-xs tabular-nums text-text-dim shrink-0">
          {count} matches
        </span>
      </header>
      {children}
    </section>
  );
}

/** Reset the §N counter at the top of each blended-render. Called by the
 *  parent component immediately before rendering the BlendedGroup tree. */
function resetBlendedGroupIndex() {
  _blendedGroupIndex = 0;
}

// ─── SYMBOL KIND DASHBOARD ─────────────────────────────────────────────────

function SymbolKindDashboard({
  symbols,
  corpusId,
}: {
  symbols: SymbolInfo[] | null;
  corpusId: string;
}) {
  void corpusId; // accept the prop to keep API surface stable
  if (symbols === null)
    return (
      <Tile title="KIND DASHBOARD">
        <LoadingRow />
      </Tile>
    );
  if (symbols.length === 0)
    return (
      <Tile title="KIND DASHBOARD" subtitle="0">
        <p className="font-sans text-xs tracking-[0.08em] text-text-dim">
          No symbols indexed
        </p>
      </Tile>
    );

  const counts = new Map<string, number>();
  for (const s of symbols) counts.set(s.kind, (counts.get(s.kind) ?? 0) + 1);
  const total = symbols.length;
  const rows = Array.from(counts.entries())
    .map(([k, c]) => ({ kind: k, count: c, pct: (c / total) * 100 }))
    .sort((a, b) => b.count - a.count);

  return (
    <Tile title="KIND DASHBOARD" subtitle={`${total} SYMBOLS`}>
      <div className="grid grid-cols-2 md:grid-cols-4 gap-0 -m-[1px]">
        {rows.slice(0, 8).map(({ kind, count, pct }) => (
          <div
            key={kind}
            className="border border-border-soft bg-surface px-3 py-2 -m-[1px] flex flex-col"
          >
            <div className="flex items-baseline justify-between">
              <span className="font-mono text-xs font-bold uppercase tracking-[0.08em] text-accent">
                {kind}
              </span>
              <span className="font-mono text-base font-bold tabular-nums text-text">
                {count}
              </span>
            </div>
            <div className="mt-1.5 h-1.5 border border-border-soft bg-surface-overlay overflow-hidden">
              <div
                className="h-full bg-accent"
                style={{ width: `${pct}%` }}
              />
            </div>
            <span className="font-mono text-mono-micro tabular-nums text-text-dim mt-0.5">
              {pct.toFixed(1)}%
            </span>
          </div>
        ))}
      </div>
    </Tile>
  );
}

// ─── CONFIDENCE RIBBON TILE (bridge landing) ───────────────────────────────

function ConfidenceRibbonTile({
  bridges,
}: {
  bridges: BridgeLink[] | null;
}) {
  if (bridges === null)
    return (
      <Tile title="CONFIDENCE">
        <LoadingRow />
      </Tile>
    );
  if (bridges.length === 0)
    return (
      <Tile title="CONFIDENCE" subtitle="0">
        <p className="font-sans text-xs tracking-[0.08em] text-text-dim">
          No bridges detected
        </p>
      </Tile>
    );

  // 5 buckets: 0-20, 20-40, 40-60, 60-80, 80-100.
  const buckets = [0, 0, 0, 0, 0];
  for (const b of bridges) {
    const i = Math.min(4, Math.floor(b.confidence * 5));
    buckets[i]++;
  }
  const max = Math.max(...buckets, 1);
  const labels = ["0-20", "20-40", "40-60", "60-80", "80+"];
  return (
    <Tile title="CONFIDENCE DISTRIBUTION" subtitle={`${bridges.length}`}>
      <div className="flex items-end gap-1 h-20">
        {buckets.map((count, i) => {
          const pct = (count / max) * 100;
          return (
            <div
              key={i}
              className="flex-1 flex flex-col items-center gap-1 min-w-0"
            >
              <span className="font-mono text-xs tabular-nums text-text">
                {count}
              </span>
              <div className="w-full border border-border-soft bg-surface-overlay flex-1 flex items-end overflow-hidden">
                <div
                  className={cn(
                    "w-full",
                    i === 4 ? "bg-success" : i >= 2 ? "bg-accent" : "bg-warning",
                  )}
                  style={{ height: `${pct}%` }}
                />
              </div>
              <span className="font-mono text-mono-micro tracking-[0.08em] text-text-dim">
                {labels[i]}
              </span>
            </div>
          );
        })}
      </div>
    </Tile>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// MODE-SPECIFIC RESULT VIEWS
// ─────────────────────────────────────────────────────────────────────────────

// ─── SURVEY RESULTS ───────────────────────────────────────────────────────

function SurveyResults({
  results,
  compact,
  onOpenDetail,
}: {
  results: SearchResult[];
  compact: boolean;
  onOpenDetail: (r: SearchResult) => void;
}) {
  // Score-distribution histogram (10 buckets).
  const histogram = useMemo(() => {
    const buckets = new Array(10).fill(0);
    for (const r of results) {
      const i = Math.min(9, Math.floor(r.score * 10));
      buckets[i]++;
    }
    return buckets;
  }, [results]);
  const maxBucket = Math.max(...histogram, 1);

  // Heading-path facets — group by the first segment.
  const facets = useMemo(() => {
    const m = new Map<string, number>();
    for (const r of results) {
      const root = r.heading_path[0] ?? "(root)";
      m.set(root, (m.get(root) ?? 0) + 1);
    }
    return Array.from(m.entries())
      .map(([root, count]) => ({ root, count }))
      .sort((a, b) => b.count - a.count);
  }, [results]);

  const [activeFacet, setActiveFacet] = useState<string | null>(null);

  const visible = useMemo(() => {
    if (!activeFacet) return results;
    return results.filter(
      (r) => (r.heading_path[0] ?? "(root)") === activeFacet,
    );
  }, [results, activeFacet]);

  return (
    <div className="flex flex-col gap-3">
      {/* Score histogram strip */}
      <div className="border border-border-soft bg-surface">
        <div className="flex items-center justify-between border-b-2 border-border bg-surface-overlay px-2 py-1">
          <span className="font-sans text-xs font-bold tracking-[0.08em] text-text">
            Score distribution
          </span>
          <span className="font-mono text-xs tabular-nums text-text-dim">
            {results.length} MATCHES
          </span>
        </div>
        <div className="flex items-end h-12 gap-[2px] p-1">
          {histogram.map((count, i) => (
            <div
              key={i}
              title={`${i * 10}-${(i + 1) * 10}% · ${count}`}
              className="flex-1 flex items-end border border-border-soft bg-surface-overlay overflow-hidden"
            >
              <div
                className={cn(
                  "w-full",
                  i >= 7 ? "bg-success" : i >= 4 ? "bg-accent" : "bg-text-muted",
                )}
                style={{
                  height: `${(count / maxBucket) * 100}%`,
                }}
              />
            </div>
          ))}
        </div>
      </div>

      {/* Two-column layout: facets + results */}
      <div className="flex gap-3 min-h-0">
        {facets.length > 1 && (
          <aside className="w-44 shrink-0 border border-border-soft bg-surface self-start">
            <div className="border-b-2 border-border bg-surface-overlay px-2 py-1">
              <span className="font-mono text-xs font-bold uppercase tracking-[0.08em] text-text">
                FACETS
              </span>
            </div>
            <button
              onClick={() => setActiveFacet(null)}
              className={cn(
                "w-full flex items-center justify-between border-b-2 border-border px-2 py-1 cursor-pointer transition-colors duration-150 ease-out",
                activeFacet === null
                  ? "bg-accent text-[var(--color-accent-fg-on)]"
                  : "bg-surface text-text hover:bg-surface-overlay",
              )}
            >
              <span className="font-mono text-xs uppercase tracking-[0.08em] truncate">
                ALL
              </span>
              <span className="font-mono text-xs tabular-nums shrink-0">
                {results.length}
              </span>
            </button>
            <div className="max-h-72 overflow-y-auto">
              {facets.slice(0, 12).map(({ root, count }) => (
                <button
                  key={root}
                  onClick={() =>
                    setActiveFacet(activeFacet === root ? null : root)
                  }
                  className={cn(
                    "w-full flex items-center justify-between border-b-2 border-border last:border-b-0 px-2 py-1 cursor-pointer transition-colors duration-150 ease-out text-left",
                    activeFacet === root
                      ? "bg-accent text-[var(--color-accent-fg-on)]"
                      : "bg-surface text-text hover:bg-surface-overlay",
                  )}
                >
                  <span className="font-mono text-xs tracking-[0.08em] truncate">
                    {root}
                  </span>
                  <span className="font-mono text-xs tabular-nums shrink-0">
                    {count}
                  </span>
                </button>
              ))}
            </div>
          </aside>
        )}

        <div className="flex-1 min-w-0">
          <ResultSection count={visible.length} label="SURVEY MATCHES">
            <div
              className={cn("flex flex-col", compact ? "gap-0" : "gap-2")}
            >
              {visible.map((r, i) => (
                <SurveyCard
                  key={`${r.content_id}-${i}`}
                  result={r}
                  compact={compact}
                  onClick={() => onOpenDetail(r)}
                />
              ))}
            </div>
          </ResultSection>
        </div>
      </div>
    </div>
  );
}

// ─── SYMBOLS RESULTS ──────────────────────────────────────────────────────

function SymbolsResults({
  symbols,
  compact,
  onOpenDetail,
}: {
  symbols: SymbolInfo[];
  compact: boolean;
  onOpenDetail: (s: SymbolInfo) => void;
}) {
  // Kind-count strip
  const kindCounts = useMemo(() => {
    const m = new Map<string, number>();
    for (const s of symbols) m.set(s.kind, (m.get(s.kind) ?? 0) + 1);
    return Array.from(m.entries())
      .map(([k, c]) => ({ kind: k, count: c }))
      .sort((a, b) => b.count - a.count);
  }, [symbols]);

  const [activeKinds, setActiveKinds] = useState<Set<string>>(new Set());
  const visible = useMemo(() => {
    if (activeKinds.size === 0) return symbols;
    return symbols.filter((s) => activeKinds.has(s.kind));
  }, [symbols, activeKinds]);

  // Always-visible inline preview (first match by default).
  const [previewed, setPreviewed] = useState<SymbolInfo | null>(symbols[0] ?? null);
  useEffect(() => {
    if (visible.length === 0) setPreviewed(null);
    else if (!previewed || !visible.some((s) => s.id === previewed.id)) {
      setPreviewed(visible[0]);
    }
  }, [visible, previewed]);

  function toggleKind(k: string) {
    setActiveKinds((prev) => {
      const next = new Set(prev);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });
  }

  return (
    <div className="flex flex-col gap-3 h-full min-h-0">
      {/* Kind-count strip */}
      <div className="border border-border-soft bg-surface shrink-0">
        <div className="flex items-center justify-between border-b-2 border-border bg-surface-overlay px-2 py-1">
          <span className="font-mono text-xs font-bold uppercase tracking-[0.08em] text-text">Kind breakdown</span>
          <span className="font-mono text-xs tabular-nums text-text-dim">
            {visible.length} / {symbols.length} SYMBOLS
          </span>
        </div>
        <div className="flex flex-wrap gap-0 -m-[1px]">
          {kindCounts.map(({ kind, count }) => {
            const active = activeKinds.has(kind);
            return (
              <button
                key={kind}
                onClick={() => toggleKind(kind)}
                className={cn(
                  "border border-border px-3 py-1.5 cursor-pointer transition-colors duration-150 ease-out -m-[1px] flex items-baseline gap-1.5",
                  active
                    ? "bg-accent text-[var(--color-accent-fg-on)] z-10 relative"
                    : "bg-surface text-text hover:bg-surface-overlay",
                )}
              >
                <span className="font-mono text-xs font-bold uppercase tracking-[0.08em]">
                  {kind}
                </span>
                <span className="font-mono text-sm font-bold tabular-nums">
                  {count}
                </span>
              </button>
            );
          })}
        </div>
      </div>

      {/* Two-pane layout: matches list + always-visible preview */}
      <div className="flex gap-3 flex-1 min-h-0">
        <div className="w-[44%] shrink-0 overflow-y-auto">
          <ResultSection count={visible.length} label="MATCHES">
            <div className={cn("flex flex-col", compact ? "gap-0" : "gap-2")}>
              {visible.map((s) => {
                const isPreviewed = previewed?.id === s.id;
                return (
                  <div
                    key={s.id}
                    onClick={() => setPreviewed(s)}
                    onDoubleClick={() => onOpenDetail(s)}
                    className={cn(
                      "cursor-pointer",
                      isPreviewed && "ring-1 ring-accent border-accent",
                    )}
                    title={isPreviewed ? "Double-click for full source" : "Click to preview"}
                  >
                    <SymbolCard
                      symbol={s}
                      compact={compact}
                      onClick={() => setPreviewed(s)}
                    />
                  </div>
                );
              })}
            </div>
          </ResultSection>
        </div>

        <div className="flex-1 min-w-0 overflow-y-auto">
          {previewed ? (
            <div className="border border-border-soft bg-surface">
              <div className="flex items-center justify-between border-b-2 border-border bg-surface-overlay px-3 py-2 sticky top-0 z-10">
                <span className="font-mono text-xs font-bold uppercase tracking-[0.08em] text-accent">
                  PREVIEW
                </span>
                <button
                  onClick={() => onOpenDetail(previewed)}
                  className="border border-border bg-surface px-2 py-0.5 font-mono text-xs font-bold uppercase tracking-[0.08em] text-text hover:bg-surface-overlay hover:text-text cursor-pointer transition-colors duration-150 ease-out"
                >Full source →</button>
              </div>
              <div className="p-3 space-y-2">
                <div className="font-mono text-xs font-bold text-text break-words">
                  {previewed.signature}
                </div>
                <div className="font-mono text-xs tracking-[0.08em] text-text-dim">
                  {previewed.module_path}
                </div>
                <div className="font-mono text-xs text-text-dim break-all">
                  {previewed.file_path}
                </div>
                {previewed.doc_comment && (
                  <div className="border-l-2 border-accent bg-surface-overlay px-2 py-1.5 font-mono text-mono-mini text-text-muted whitespace-pre-wrap">
                    {previewed.doc_comment}
                  </div>
                )}
              </div>
            </div>
          ) : (
            <div className="border border-dotted border-border bg-surface px-3 py-6 text-center font-sans text-xs tracking-[0.08em] text-text-dim">
              Select a symbol to preview
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

// ─── BRIDGE RESULTS ───────────────────────────────────────────────────────

function BridgeResults({
  bridges,
  corpusId,
  onOpenDetail,
}: {
  bridges: BridgeLink[];
  corpusId: string;
  onOpenDetail: (b: BridgeLink) => void;
}) {
  void onOpenDetail;
  // Group counts by kind for the summary strip
  const kindCounts = useMemo(() => {
    const m = new Map<string, number>();
    for (const b of bridges) m.set(b.kind, (m.get(b.kind) ?? 0) + 1);
    return Array.from(m.entries())
      .map(([k, c]) => ({ kind: k, count: c }))
      .sort((a, b) => b.count - a.count);
  }, [bridges]);

  const [activeKind, setActiveKind] = useState<string | null>(null);
  const [expandedIdx, setExpandedIdx] = useState<number | null>(null);
  const [excerpts, setExcerpts] = useState<{
    exportSrc: string | null;
    importSrc: string | null;
    loading: boolean;
  }>({ exportSrc: null, importSrc: null, loading: false });

  const visible = useMemo(() => {
    if (!activeKind) return bridges;
    return bridges.filter((b) => b.kind === activeKind);
  }, [bridges, activeKind]);

  useEffect(() => {
    if (expandedIdx === null) {
      setExcerpts({ exportSrc: null, importSrc: null, loading: false });
      return;
    }
    const link = visible[expandedIdx];
    if (!link) return;
    let cancelled = false;
    setExcerpts({ exportSrc: null, importSrc: null, loading: true });
    Promise.allSettled([
      invoke<string>("read_source_excerpt", {
        corpusId,
        filePath: link.export_file,
        lineStart: link.export_line,
        lineEnd: link.export_line,
      }),
      invoke<string>("read_source_excerpt", {
        corpusId,
        filePath: link.import_file,
        lineStart: link.import_line,
        lineEnd: link.import_line,
      }),
    ]).then(([e, i]) => {
      if (cancelled) return;
      setExcerpts({
        exportSrc:
          e.status === "fulfilled" ? e.value : "// (could not read source)",
        importSrc:
          i.status === "fulfilled" ? i.value : "// (could not read source)",
        loading: false,
      });
    });
    return () => {
      cancelled = true;
    };
  }, [expandedIdx, visible, corpusId]);

  const total = bridges.length || 1;
  return (
    <div className="flex flex-col gap-3 h-full min-h-0">
      {/* Kind summary strip — proportional blocks */}
      <div className="border border-border-soft bg-surface shrink-0">
        <div className="flex items-center justify-between border-b-2 border-border bg-surface-overlay px-2 py-1">
          <span className="font-mono text-xs font-bold uppercase tracking-[0.08em] text-text">Bridge surface</span>
          <span className="font-mono text-xs tabular-nums text-text-dim">
            {visible.length} / {bridges.length} LINKS
          </span>
        </div>
        <div className="flex items-stretch h-14 -mx-[1px]">
          {kindCounts.map(({ kind, count }) => {
            const active = kind === activeKind;
            const pct = (count / total) * 100;
            return (
              <button
                key={kind}
                onClick={() => setActiveKind(activeKind === kind ? null : kind)}
                title={`${kind} · ${count} (${pct.toFixed(1)}%)`}
                className={cn(
                  "flex flex-col items-start justify-center border border-border px-2 cursor-pointer transition-colors duration-150 ease-out -ml-[2px] first:ml-0 min-w-0",
                  active
                    ? "bg-accent text-[var(--color-accent-fg-on)] z-10 relative"
                    : "bg-surface text-text hover:bg-surface-overlay",
                )}
                style={{ width: `max(7%, ${pct}%)` }}
              >
                <span className="font-mono text-mono-micro font-bold uppercase tracking-[0.08em] opacity-70 truncate w-full">
                  {kind}
                </span>
                <span className="font-mono text-base font-bold tabular-nums leading-none mt-0.5">
                  {count}
                </span>
              </button>
            );
          })}
        </div>
      </div>

      {/* Visual rows: EXPORT — connector — IMPORT, click expands inline */}
      <div className="flex-1 min-h-0 overflow-y-auto border border-border-soft bg-surface">
        <div className="border-b-2 border-border bg-surface-overlay px-2 py-1 sticky top-0 z-10 flex items-center justify-between">
          <span className="font-mono text-xs font-bold uppercase tracking-[0.08em] text-text">Bridge links</span>
          <span className="font-mono text-xs tabular-nums text-text-dim">
            Click to expand
          </span>
        </div>
        {visible.map((b, i) => {
          const expanded = expandedIdx === i;
          return (
            <div key={`${b.kind}-${i}`} className="border-b-2 border-border">
              <button
                onClick={() => setExpandedIdx(expanded ? null : i)}
                className={cn(
                  "w-full text-left grid grid-cols-[1fr_auto_1fr_auto_60px] gap-2 px-3 py-2 cursor-pointer transition-colors duration-150 ease-out items-center",
                  expanded
                    ? "bg-accent text-[var(--color-accent-fg-on)]"
                    : "bg-surface text-text hover:bg-surface-overlay",
                )}
              >
                <span className="flex items-center gap-2 min-w-0">
                  <span className="border border-border-soft px-1 font-mono text-mono-micro uppercase tracking-[0.08em] opacity-70 shrink-0">
                    {b.export_language}
                  </span>
                  <span className="font-mono text-xs font-bold truncate">
                    {b.export_symbol || b.export_binding_key}
                  </span>
                </span>
                <span className="font-mono text-xs uppercase tracking-[0.08em] opacity-70 shrink-0">
                  {b.kind}
                </span>
                <span className="flex items-center gap-2 min-w-0">
                  <span className="border border-border-soft px-1 font-mono text-mono-micro uppercase tracking-[0.08em] opacity-70 shrink-0">
                    {b.import_language}
                  </span>
                  <span className="font-mono text-xs font-bold truncate">
                    {b.import_symbol || b.import_binding_key}
                  </span>
                </span>
                <ChevronRight
                  className={cn("h-3 w-3 shrink-0 transition-colors duration-150 ease-out", expanded && "rotate-90")}
                  strokeWidth={2.5}
                />
                <span className="font-mono text-xs tabular-nums text-right shrink-0">
                  {(b.confidence * 100).toFixed(0)}%
                </span>
              </button>
              {expanded && (
                <div className="border-t-2 border-accent bg-surface-sunken p-3 grid grid-cols-2 gap-3">
                  <CodeExcerptPane
                    title="EXPORT"
                    file={b.export_file}
                    line={b.export_line}
                    source={excerpts.exportSrc}
                    loading={excerpts.loading}
                  />
                  <CodeExcerptPane
                    title="IMPORT"
                    file={b.import_file}
                    line={b.import_line}
                    source={excerpts.importSrc}
                    loading={excerpts.loading}
                  />
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function CodeExcerptPane({
  title,
  file,
  line,
  source,
  loading,
}: {
  title: string;
  file: string;
  line: number;
  source: string | null;
  loading: boolean;
}) {
  const tail = file.replace(/\\/g, "/").split("/").slice(-2).join("/");
  return (
    <div className="border border-border-soft bg-surface">
      <div className="flex items-center justify-between border-b-2 border-border bg-surface-overlay px-2 py-1">
        <span className="font-mono text-xs font-bold uppercase tracking-[0.08em] text-accent">
          {title}
        </span>
        <span className="font-mono text-xs text-text-dim truncate ml-2">
          {tail}:{line}
        </span>
      </div>
      <pre className="bg-surface-sunken px-3 py-2 font-mono text-mono-mini leading-relaxed text-text whitespace-pre overflow-x-auto m-0 max-h-48 overflow-y-auto">
        {loading
          ? "LOADING_"
          : (source ?? "// (no source available)")}
      </pre>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// LANDING TILES — explorable cold-load surface for the Code Intelligence view.
// ─────────────────────────────────────────────────────────────────────────────

interface LandingTilesProps {
  corpus: CorpusInfo;
  onJumpToFile: (filePath: string) => void;
  onJumpToBridgeKind: (kind: string) => void;
}

function LandingTiles({
  corpus,
  onJumpToFile,
  onJumpToBridgeKind,
}: LandingTilesProps) {
  const [files, setFiles] = useState<FileInfo[] | null>(null);
  const [bridges, setBridges] = useState<BridgeLink[] | null>(null);
  const [coherence, setCoherence] = useState<CoherenceEvent[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    setFiles(null);
    setBridges(null);
    setCoherence(null);

    Promise.allSettled([
      invoke<FileInfo[]>("list_corpus_files", { corpusId: corpus.id }),
      invoke<BridgeLink[]>("bridge_query", {
        corpusId: corpus.id,
        query: null,
        kind: null,
        sourceLanguage: null,
        filePath: null,
        limit: 500,
      }),
      invoke<CoherenceEvent[]>("recent_coherence_events", {
        limit: 10,
        sinceMs: null,
      }),
    ]).then(([f, b, c]) => {
      if (cancelled) return;
      setFiles(f.status === "fulfilled" ? f.value : []);
      setBridges(b.status === "fulfilled" ? b.value : []);
      setCoherence(c.status === "fulfilled" ? c.value : []);
    });
    return () => {
      cancelled = true;
    };
  }, [corpus.id]);

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-3">
      <StructureTile corpus={corpus} files={files} />
      <BridgesTile bridges={bridges} onJumpToKind={onJumpToBridgeKind} />
      <HotFilesTile files={files} corpus={corpus} onJumpToFile={onJumpToFile} />
      <RecentChangesTile events={coherence} corpus={corpus} />
    </div>
  );
}

// ─── STRUCTURE TILE ────────────────────────────────────────────────────────

function StructureTile({
  corpus,
  files,
}: {
  corpus: CorpusInfo;
  files: FileInfo[] | null;
}) {
  const langMix = useMemo(() => {
    if (!files) return [];
    const total = files.reduce((s, f) => s + f.section_count, 0);
    if (total === 0) return [];
    const byExt = new Map<string, number>();
    for (const f of files) {
      const ext = (f.path.split(".").pop() ?? "other").toLowerCase();
      byExt.set(ext, (byExt.get(ext) ?? 0) + f.section_count);
    }
    return Array.from(byExt.entries())
      .map(([ext, count]) => ({ ext, count, pct: (count / total) * 100 }))
      .sort((a, b) => b.count - a.count)
      .slice(0, 5);
  }, [files]);

  return (
    <Tile title="STRUCTURE" subtitle={`${corpus.files_indexed.toLocaleString()} files`}>
      <StatRow label="files" value={corpus.files_indexed} max={corpus.files_indexed} />
      <StatRow
        label="sections"
        value={corpus.sections_count}
        max={corpus.sections_count}
      />
      <StatRow
        label="symbols"
        value={corpus.symbols_count ?? 0}
        max={Math.max(corpus.sections_count, corpus.symbols_count ?? 0)}
      />
      <StatRow
        label="vectors"
        value={corpus.embeddings_count}
        max={Math.max(corpus.sections_count, corpus.embeddings_count)}
      />

      {langMix.length > 0 && (
        <div className="mt-3 pt-3 border-t-2 border-border">
          <div className="font-mono text-xs font-bold uppercase tracking-[0.08em] text-text-dim mb-1.5">Lang mix</div>
          <div className="flex h-3 border border-border-soft bg-surface-overlay overflow-hidden">
            {langMix.map(({ ext, pct }, i) => (
              <div
                key={ext}
                className={cn("h-full", i === 0 ? "bg-accent" : "bg-text-muted")}
                style={{
                  width: `${pct}%`,
                  opacity: 1 - i * 0.18,
                }}
                title={`${ext}: ${pct.toFixed(0)}%`}
              />
            ))}
          </div>
          <div className="mt-1.5 flex flex-wrap gap-x-2 gap-y-0.5 font-mono text-xs text-text-dim">
            {langMix.map(({ ext, pct }) => (
              <span key={ext}>
                .{ext} <span className="tabular-nums">{pct.toFixed(0)}%</span>
              </span>
            ))}
          </div>
        </div>
      )}
    </Tile>
  );
}

function StatRow({
  label,
  value,
  max,
}: {
  label: string;
  value: number;
  max: number;
}) {
  const pct = max > 0 ? Math.min(100, (value / max) * 100) : 0;
  return (
    <div className="flex items-center gap-2">
      <span className="font-mono text-xs tracking-[0.08em] text-text-dim w-16 shrink-0">
        {label}
      </span>
      <span className="font-mono text-xs font-bold tabular-nums text-text w-16 shrink-0 text-right">
        {value.toLocaleString()}
      </span>
      <div className="flex-1 h-2 border border-border-soft bg-surface-overlay overflow-hidden">
        <div className="h-full bg-accent" style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
}

// ─── BRIDGES TILE ──────────────────────────────────────────────────────────

function BridgesTile({
  bridges,
  onJumpToKind,
}: {
  bridges: BridgeLink[] | null;
  onJumpToKind: (kind: string) => void;
}) {
  const grouped = useMemo(() => {
    if (!bridges) return null;
    const m = new Map<string, number>();
    for (const b of bridges) m.set(b.kind, (m.get(b.kind) ?? 0) + 1);
    const arr = Array.from(m.entries())
      .map(([kind, count]) => ({ kind, count }))
      .sort((a, b) => b.count - a.count);
    const total = bridges.length;
    return { rows: arr, total };
  }, [bridges]);

  if (bridges === null) return <Tile title="BRIDGES"><LoadingRow /></Tile>;
  if (grouped && grouped.total === 0) {
    return (
      <Tile title="BRIDGES" subtitle="0">
        <p className="font-sans text-xs tracking-[0.08em] text-text-dim">
          No cross-language links detected
        </p>
      </Tile>
    );
  }

  return (
    <Tile
      title="BRIDGES"
      subtitle={grouped ? `${grouped.total}` : ""}
      hint="click a kind to filter"
    >
      <div className="flex flex-col">
        {grouped?.rows.map(({ kind, count }) => {
          const pct = grouped.total > 0 ? (count / grouped.total) * 100 : 0;
          return (
            <button
              key={kind}
              onClick={() => onJumpToKind(kind)}
              className="flex items-center gap-2 px-1 py-1 border-b-2 border-border last:border-b-0 hover:bg-surface-overlay hover:text-text cursor-pointer transition-colors duration-150 ease-out -mx-1"
            >
              <span className="font-mono text-xs font-bold uppercase tracking-[0.08em] w-32 shrink-0 text-left">
                {kind}
              </span>
              <span className="font-mono text-xs font-bold tabular-nums w-10 shrink-0 text-right">
                {count}
              </span>
              <div className="flex-1 h-2 border border-border-soft bg-surface-overlay overflow-hidden">
                <div className="h-full bg-accent" style={{ width: `${pct}%` }} />
              </div>
            </button>
          );
        })}
      </div>
    </Tile>
  );
}

// ─── HOT FILES TILE ────────────────────────────────────────────────────────

function HotFilesTile({
  files,
  corpus,
  onJumpToFile,
}: {
  files: FileInfo[] | null;
  corpus: CorpusInfo;
  onJumpToFile: (filePath: string) => void;
}) {
  const top = useMemo(() => {
    if (!files) return null;
    return [...files]
      .sort((a, b) => b.section_count - a.section_count)
      .slice(0, 12);
  }, [files]);

  const max = top && top.length > 0 ? top[0].section_count : 1;

  if (files === null) return <Tile title="HOT FILES"><LoadingRow /></Tile>;
  if (!top || top.length === 0) {
    return (
      <Tile title="HOT FILES" subtitle="0">
        <p className="font-sans text-xs tracking-[0.08em] text-text-dim">
          No indexed files
        </p>
      </Tile>
    );
  }

  return (
    <Tile
      title="HOT FILES"
      subtitle={`TOP ${top.length}`}
      hint="click to see symbols in file"
    >
      <div className="flex flex-col">
        {top.map((f) => {
          const pct = max > 0 ? (f.section_count / max) * 100 : 0;
          const tail = corpusRelative(f.path, corpus);
          return (
            <button
              key={f.path}
              onClick={() => onJumpToFile(f.path)}
              title={f.path}
              className="flex items-center gap-2 px-1 py-1 border-b-2 border-border last:border-b-0 hover:bg-surface-overlay hover:text-text cursor-pointer transition-colors duration-150 ease-out -mx-1"
            >
              <span className="font-mono text-mono-mini truncate flex-1 text-left">
                {tail}
              </span>
              <div className="w-20 h-2 border border-border-soft bg-surface-overlay overflow-hidden shrink-0">
                <div className="h-full bg-accent" style={{ width: `${pct}%` }} />
              </div>
              <span className="font-mono text-xs font-bold tabular-nums w-10 shrink-0 text-right">
                {f.section_count}
              </span>
            </button>
          );
        })}
      </div>
    </Tile>
  );
}

// ─── RECENT CHANGES TILE ───────────────────────────────────────────────────

const COHERENCE_GLYPH: Record<CoherenceKind, string> = {
  created: "+",
  modified: "~",
  removed: "−",
};

function RecentChangesTile({
  events,
  corpus,
}: {
  events: CoherenceEvent[] | null;
  corpus: CorpusInfo;
}) {
  const filtered = useMemo(() => {
    if (!events) return null;
    return events
      .filter((e) => e.corpus_id === corpus.id)
      .slice(0, 10);
  }, [events, corpus.id]);

  if (events === null) return <Tile title="RECENT CHANGES"><LoadingRow /></Tile>;
  if (!filtered || filtered.length === 0) {
    return (
      <Tile title="RECENT CHANGES" subtitle="0">
        <p className="font-sans text-xs tracking-[0.08em] text-text-dim">
          No recent file changes
        </p>
      </Tile>
    );
  }

  const now = Date.now();
  return (
    <Tile title="RECENT CHANGES" subtitle={`${filtered.length}`}>
      <div className="flex flex-col">
        {filtered.map((ev, i) => (
          <div
            key={`${ev.timestamp_ms}-${i}`}
            className="flex items-center gap-2 px-1 py-1 border-b-2 border-border last:border-b-0"
          >
            <span className="font-mono text-xs font-bold w-4 shrink-0">
              {COHERENCE_GLYPH[ev.kind]}
            </span>
            <span
              className="font-mono text-mono-mini text-text truncate flex-1"
              title={ev.path}
            >
              {corpusRelative(ev.path, corpus)}
            </span>
            <span className="font-mono text-xs tabular-nums text-text-dim shrink-0">
              {relative(now, ev.timestamp_ms)}
            </span>
          </div>
        ))}
      </div>
    </Tile>
  );
}

// ─── EMPTY-CORPUS TILE ─────────────────────────────────────────────────────

function EmptyCorpusTile() {
  return (
    <div className="border border-border-soft bg-surface p-8 text-center">
      <h3 className="font-sans text-base font-bold tracking-[0.08em] text-text">
        Add a project to begin
      </h3>
      <p className="mt-3 font-sans text-xs tracking-[0.08em] text-text-dim">
        Open the Projects tab and add a directory — ministr will index it for survey, symbols, and bridge.
      </p>
    </div>
  );
}

// ─── SHARED TILE PRIMITIVES ────────────────────────────────────────────────

function Tile({
  title,
  subtitle,
  hint,
  children,
}: {
  title: string;
  subtitle?: string;
  hint?: string;
  children: React.ReactNode;
}) {
  // Tile titles arrive UPPERCASE from legacy callers; sentence-case them
  // for the field-manual aesthetic.
  const sentence = /^[A-Z][A-Z0-9\s\-—·]+$/.test(title)
    ? title.charAt(0) + title.slice(1).toLowerCase()
    : title;
  return (
    <section className="border border-border-soft bg-surface flex flex-col">
      <header className="flex items-baseline justify-between gap-2 border-b border-border-soft bg-surface-overlay px-3 py-2">
        <h3 className="font-sans text-base font-bold text-text">
          {sentence}
        </h3>
        <div className="flex items-center gap-2">
          {hint && (
            <span className="font-sans text-xs italic text-text-dim">
              {hint}
            </span>
          )}
          {subtitle && (
            <span className="font-mono text-xs tabular-nums text-text-dim">
              {subtitle}
            </span>
          )}
        </div>
      </header>
      <div className="p-3 flex-1">{children}</div>
    </section>
  );
}

function LoadingRow() {
  return (
    <p className="font-sans text-base italic text-text-dim">
      Loading<span className="ministr-blink">_</span>
    </p>
  );
}

// ─── RESULT PRIMITIVES ─────────────────────────────────────────────────────

function ResultSection({
  count,
  label,
  children,
}: {
  count: number;
  label: string;
  children: React.ReactNode;
}) {
  const sentence = /^[A-Z][A-Z0-9\s\-—·]+$/.test(label)
    ? label.charAt(0) + label.slice(1).toLowerCase()
    : label;
  return (
    <div className="space-y-2">
      <div className="flex items-baseline justify-between border-b border-border-soft bg-surface-overlay px-3 py-2">
        <h3 className="font-sans text-base font-bold text-text">
          {sentence}
        </h3>
        <span className="font-mono text-xs tabular-nums text-text-dim">
          {count}
        </span>
      </div>
      {children}
    </div>
  );
}

/** Strip the corpus root from a `content_id` like
 *  `D:/code/foo/src/lib.rs#mod::Bar:c0` so the small mono badge shows
 *  the project-relative path plus the section anchor. Falls back to the
 *  original id (basename + anchor) when no corpus is available. */
function shortContentId(
  contentId: string,
  corpus: CorpusInfo | null,
): string {
  const norm = contentId.replace(/\\/g, "/");
  // Symbol ids (`sym-…`) carry the file path before `::`. Preserve that.
  const stripped = norm.replace(/^sym-/, "");
  const hashIdx = stripped.indexOf("#");
  const colonIdx = stripped.indexOf("::");
  const splitIdx =
    hashIdx >= 0 && (colonIdx < 0 || hashIdx < colonIdx)
      ? hashIdx
      : colonIdx;
  if (splitIdx < 0) {
    return corpusRelative(stripped, corpus);
  }
  const filePart = stripped.slice(0, splitIdx);
  const tail = stripped.slice(splitIdx);
  const rel = corpusRelative(filePart, corpus);
  return `${rel}${tail}`;
}

// ─── SURVEY CARD ───────────────────────────────────────────────────────────

function SurveyCard({
  result,
  compact,
  corpus,
  onClick,
}: {
  result: SearchResult;
  compact?: boolean;
  /** Optional. When provided, the card's small mono badge shows a
   *  corpus-relative path. Falls back to last-2-segments otherwise. */
  corpus?: CorpusInfo | null;
  onClick: () => void;
}) {
  const pct = Math.max(0, Math.min(100, result.score * 100));
  const shortId = shortContentId(result.content_id, corpus ?? null);
  const excerptLines = (result.text ?? "")
    .split("\n")
    .filter((l) => l.trim().length > 0)
    .slice(0, 3)
    .join("\n");

  if (compact) {
    const oneLine = (result.text ?? "")
      .replace(/\s+/g, " ")
      .trim()
      .slice(0, 80);
    return (
      <button
        onClick={onClick}
        className="text-left flex items-center gap-2 border-b border-border-soft bg-surface px-3 py-1.5 cursor-pointer transition-colors duration-150 ease-out hover:bg-surface-overlay hover:text-text"
      >
        <span className="font-mono text-xs font-semibold tabular-nums w-10 shrink-0 text-text-dim">
          {pct.toFixed(0)}%
        </span>
        <div className="w-16 h-1.5 border border-border-soft bg-surface-overlay overflow-hidden shrink-0">
          <div className="h-full bg-accent" style={{ width: `${pct}%` }} />
        </div>
        <span className="font-mono text-xs truncate w-48 shrink-0 text-text-dim">
          {shortId}
        </span>
        <span className="font-sans text-sm text-text-dim truncate flex-1">
          {oneLine}
        </span>
      </button>
    );
  }

  return (
    <button
      onClick={onClick}
      className="text-left border border-border-soft bg-surface cursor-pointer transition-colors duration-150 ease-out hover:border-border hover:bg-surface-overlay"
    >
      <div className="flex items-center gap-2 border-b border-border-soft bg-surface-overlay px-3 py-1.5">
        <span className="font-mono text-xs font-bold tabular-nums text-text w-10 shrink-0">
          {pct.toFixed(0)}%
        </span>
        <div className="w-24 h-2 border border-border-soft bg-surface overflow-hidden shrink-0">
          <div className="h-full bg-accent" style={{ width: `${pct}%` }} />
        </div>
        <span className="font-mono text-mono-mini text-text truncate">
          {shortId}
        </span>
      </div>

      {result.heading_path.length > 0 && (
        <div className="flex items-center gap-1 px-2 py-1 border-b-2 border-border font-mono text-xs uppercase tracking-[0.08em] text-text-dim flex-wrap">
          {result.heading_path.map((h, j) => (
            <span key={j} className="flex items-center gap-1">
              {j > 0 && (
                <ChevronRight className="h-2.5 w-2.5" strokeWidth={2.5} />
              )}
              <span className="text-text">{h}</span>
            </span>
          ))}
        </div>
      )}

      {excerptLines.length > 0 && (
        <pre className="border-l-2 border-accent bg-surface-sunken px-3 py-2 font-mono text-mono-mini leading-relaxed text-text whitespace-pre-wrap line-clamp-3 m-0">
          {excerptLines}
        </pre>
      )}
    </button>
  );
}

// ─── SYMBOL CARD ───────────────────────────────────────────────────────────

function SymbolCard({
  symbol,
  compact,
  onClick,
}: {
  symbol: SymbolInfo;
  compact?: boolean;
  onClick: () => void;
}) {
  const fileName = symbol.file_path.split(/[\\/]/).pop() ?? symbol.file_path;

  if (compact) {
    const sig = (symbol.signature ?? "")
      .replace(/\s+/g, " ")
      .trim()
      .slice(0, 100);
    return (
      <button
        onClick={onClick}
        className="text-left flex items-center gap-2 border-b-2 border-border bg-surface px-2 py-1.5 cursor-pointer transition-colors duration-150 ease-out hover:bg-surface-overlay hover:text-text hover:translate-x-[2px]"
      >
        <span className="font-mono text-xs font-bold uppercase tracking-[0.08em] text-accent w-12 shrink-0">
          {symbol.kind}
        </span>
        <span className="font-mono text-xs font-bold truncate w-48 shrink-0">
          {symbol.name}
        </span>
        <span className="font-mono text-xs opacity-70 truncate flex-1">
          {sig}
        </span>
        <span className="font-mono text-xs opacity-70 shrink-0">
          {fileName}
        </span>
      </button>
    );
  }

  return (
    <button
      onClick={onClick}
      className="text-left border border-border bg-surface cursor-pointer transition-colors duration-150 ease-out hover:-translate-x-[2px] hover:-translate-y-[2px] hover:shadow-md"
    >
      <div className="flex items-center gap-2 border-b-2 border-border bg-surface-overlay px-2 py-1.5">
        <span className="font-mono text-xs font-bold uppercase tracking-[0.08em] text-accent w-14 shrink-0">
          {symbol.kind}
        </span>
        <span className="font-mono text-sm font-bold text-text truncate flex-1">
          {symbol.name}
        </span>
        {symbol.visibility && (
          <span className="font-mono text-xs uppercase tracking-[0.08em] text-text-dim shrink-0">
            {symbol.visibility}
          </span>
        )}
      </div>

      {symbol.signature && (
        <pre className="border-b-2 border-border bg-surface-sunken px-3 py-2 font-mono text-mono-mini leading-relaxed text-text whitespace-pre-wrap break-words m-0">
          {symbol.signature}
        </pre>
      )}

      <div className="flex items-center gap-2 px-2 py-1 font-mono text-xs text-text-dim">
        <span className="truncate flex-1">{symbol.module_path}</span>
        <span className="shrink-0">{fileName}</span>
      </div>
    </button>
  );
}

function ViewToggle({
  label,
  active,
  disabled,
  onClick,
}: {
  label: string;
  active: boolean;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <FilterPill
      tone="sans"
      size="md"
      active={active}
      disabled={disabled}
      onClick={onClick}
      className="shrink-0"
    >
      {label}
    </FilterPill>
  );
}

function ResultRow({
  children,
  onClick,
}: {
  children: React.ReactNode;
  onClick?: () => void;
}) {
  return (
    <div
      onClick={onClick}
      className={cn(
        "flex items-center gap-2 border-b border-border-soft px-3 py-2 transition-colors duration-150 ease-out",
        onClick && "cursor-pointer hover:bg-surface-overlay hover:text-text",
      )}
    >
      {children}
    </div>
  );
}

