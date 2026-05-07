import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronRight, Download, ExternalLink, X } from "lucide-react";
import { Button } from "./ui/button";
import { cn } from "../lib/utils";
import { corpusRelative } from "../lib/path";
import { useEntityPanel } from "../hooks/useEntityPanel";
import type { BridgeLink, CorpusInfo, DaemonStatus } from "../lib/types";

/** Stable identity for a bridge link — used as the key in the multi-select
 *  set. The DB doesn't expose a numeric ID; this composite uniquely picks
 *  out a single (export_file:line ↔ import_file:line) pair within a kind. */
function bridgeKey(l: BridgeLink): string {
  return `${l.kind}|${l.export_file}:${l.export_line}|${l.import_file}:${l.import_line}`;
}

/** Trigger a browser download of `text` as `filename`. Tauri's webview
 *  honors blob: anchors so this works in the desktop app with no extra
 *  permissions; the file lands in the user's Downloads folder. */
function downloadText(filename: string, text: string, mime: string) {
  const blob = new Blob([text], { type: `${mime};charset=utf-8` });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function exportJson(rows: BridgeLink[]) {
  const ts = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
  downloadText(
    `ministr-bridges-${ts}.json`,
    JSON.stringify(rows, null, 2),
    "application/json",
  );
}

function exportCsv(rows: BridgeLink[]) {
  // RFC-4180-flavored CSV. Quote every field; double up internal quotes.
  const headers = [
    "kind",
    "confidence",
    "export_language",
    "export_symbol",
    "export_binding_key",
    "export_file",
    "export_line",
    "import_language",
    "import_symbol",
    "import_binding_key",
    "import_file",
    "import_line",
  ] as const;
  const esc = (v: unknown) => `"${String(v ?? "").replaceAll('"', '""')}"`;
  const lines = [
    headers.map(esc).join(","),
    ...rows.map((r) =>
      headers.map((h) => esc((r as unknown as Record<string, unknown>)[h])).join(","),
    ),
  ];
  const ts = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
  downloadText(`ministr-bridges-${ts}.csv`, lines.join("\n"), "text/csv");
}

interface Props {
  status: DaemonStatus;
  activeCorpusId: string | null;
  setActiveCorpusId: (id: string | null) => void;
}

const KIND_FILTERS = [
  "tauri_command",
  "tauri_event",
  "pyo3_function",
  "napi_export",
  "wasm_bindgen",
  "http_route",
  "ffi",
] as const;

const LANGUAGE_FILTERS = ["rust", "python", "typescript", "javascript"] as const;

const CONFIDENCE_HELP =
  "How certain the linker is the export and import refer to the same bridge. High = exact match (binding key, signature). Low = heuristic (name match across languages).";

interface ExcerptState {
  exportSrc: string | null;
  importSrc: string | null;
  exportStart: number | null;
  importStart: number | null;
  loading: boolean;
}

export function Bridge({ status, activeCorpusId }: Props) {
  const { openEntity } = useEntityPanel();
  const corpusId = activeCorpusId ?? status.corpora[0]?.id ?? "";
  const selectedCorpus = useMemo(
    () => status.corpora.find((c) => c.id === corpusId) ?? null,
    [status.corpora, corpusId],
  );
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<string | null>(null);
  const [language, setLanguage] = useState<string | null>(null);
  const [links, setLinks] = useState<BridgeLink[]>([]);
  const [loading, setLoading] = useState(false);
  const [selected, setSelected] = useState<BridgeLink | null>(null);
  const [excerpt, setExcerpt] = useState<ExcerptState>({
    exportSrc: null,
    importSrc: null,
    exportStart: null,
    importStart: null,
    loading: false,
  });
  const [showConfHelp, setShowConfHelp] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());

  const selectedRows = useMemo(
    () => links.filter((l) => selectedIds.has(bridgeKey(l))),
    [links, selectedIds],
  );

  const toggleRow = useCallback((l: BridgeLink) => {
    const id = bridgeKey(l);
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const toggleAll = useCallback(() => {
    setSelectedIds((prev) => {
      if (prev.size === links.length) return new Set();
      return new Set(links.map(bridgeKey));
    });
  }, [links]);

  const clearSelection = useCallback(() => setSelectedIds(new Set()), []);

  async function runQuery() {
    if (!corpusId) return;
    setLoading(true);
    try {
      const r = await invoke<BridgeLink[]>("bridge_query", {
        corpusId,
        query: query.trim() || null,
        kind,
        sourceLanguage: language,
        filePath: null,
        limit: 500,
      });
      setLinks(r);
      setSelected(null);
    } catch (e) {
      console.error("bridge_query failed", e);
      setLinks([]);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    if (!corpusId) return;
    runQuery();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [corpusId, kind, language]);

  // When the result set changes (new query, new filters, new corpus), drop
  // any selection — old keys may no longer point at any visible row.
  useEffect(() => {
    setSelectedIds(new Set());
  }, [corpusId, kind, language, links]);

  // Fetch source excerpts when a row is selected.
  useEffect(() => {
    if (!selected) {
      setExcerpt({
        exportSrc: null,
        importSrc: null,
        exportStart: null,
        importStart: null,
        loading: false,
      });
      return;
    }
    let cancelled = false;
    setExcerpt({
      exportSrc: null,
      importSrc: null,
      exportStart: Math.max(1, selected.export_line - 3),
      importStart: Math.max(1, selected.import_line - 3),
      loading: true,
    });
    Promise.allSettled([
      invoke<string>("read_source_excerpt", {
        corpusId,
        filePath: selected.export_file,
        lineStart: selected.export_line,
        lineEnd: selected.export_line,
      }),
      invoke<string>("read_source_excerpt", {
        corpusId,
        filePath: selected.import_file,
        lineStart: selected.import_line,
        lineEnd: selected.import_line,
      }),
    ]).then(([e, i]) => {
      if (cancelled) return;
      setExcerpt({
        exportSrc:
          e.status === "fulfilled" ? e.value : "// (could not read source)",
        importSrc:
          i.status === "fulfilled" ? i.value : "// (could not read source)",
        exportStart: Math.max(1, selected.export_line - 3),
        importStart: Math.max(1, selected.import_line - 3),
        loading: false,
      });
    });
    return () => {
      cancelled = true;
    };
  }, [selected, corpusId]);

  const grouped = useMemo(() => {
    const g = new Map<string, number>();
    for (const l of links) {
      g.set(l.kind, (g.get(l.kind) ?? 0) + 1);
    }
    return g;
  }, [links]);

  return (
    <div className="@container/page flex flex-col h-full gap-3 min-h-0">
      <header>
        <h2 className="font-serif text-2xl font-normal text-text leading-tight ">
          Cross-language bridge
        </h2>
        <p className="font-sans text-xs tracking-[0.05em] text-text-dim mt-1">
          Tauri · PyO3 · NAPI · wasm-bindgen · HTTP routes · raw FFI
        </p>
      </header>

      {/* Kind summary hero — full block when no row selected, ribbon when one is. */}
      <KindSummary
        grouped={grouped}
        total={links.length}
        compact={!!selected}
        activeKind={kind}
        onPick={(k) => setKind(kind === k ? null : k)}
      />

      {/* Compact filter row: kind/lang pills + query input on a single line. */}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          runQuery();
        }}
        className="flex items-center gap-2 flex-wrap"
      >
        <FilterPill
          label="ALL"
          active={kind === null}
          onClick={() => setKind(null)}
        />
        {KIND_FILTERS.map((k) => (
          <FilterPill
            key={k}
            label={k.toUpperCase()}
            count={grouped.get(k)}
            active={kind === k}
            onClick={() => setKind(kind === k ? null : k)}
          />
        ))}
        <span className="w-px h-5 bg-border opacity-50 mx-0.5" />
        <FilterPill
          label="ANY LANG"
          active={language === null}
          onClick={() => setLanguage(null)}
        />
        {LANGUAGE_FILTERS.map((l) => (
          <FilterPill
            key={l}
            label={l.toUpperCase()}
            active={language === l}
            onClick={() => setLanguage(language === l ? null : l)}
          />
        ))}
        <div className="flex items-center gap-2 ml-auto">
          <span className="font-mono text-base font-bold text-accent shrink-0">
            {">"}
          </span>
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="filter (optional)"
            className="h-9 w-56 border-2 border-border bg-surface px-2 text-xs font-mono text-text placeholder:text-text-dim focus:outline-none focus:bg-surface-overlay focus:text-text transition-none"
          />
          <Button type="submit" disabled={loading} size="sm">
            {loading ? "…" : "RUN"}
          </Button>
        </div>
      </form>

      {/* Selection bar — appears once at least one row is checked. Sits on
          top of the table so it's always reachable while scrolling. */}
      {selectedIds.size > 0 && (
        <div className="flex items-center gap-3 border border-accent bg-surface-overlay px-3 py-2 shrink-0">
          <span className="font-sans text-sm text-text">
            <span className="font-mono tabular-nums font-semibold">
              {selectedIds.size}
            </span>{" "}
            of {links.length} selected
          </span>
          <span className="w-px h-5 bg-border-soft" />
          <button
            onClick={() => exportJson(selectedRows)}
            className="inline-flex items-center gap-1.5 border border-border-soft bg-surface px-2 py-0.5 font-sans text-sm font-medium text-text-muted hover:text-text hover:border-border cursor-pointer transition-none"
            style={{ borderRadius: "var(--radius-button)" }}
          >
            <Download className="h-3.5 w-3.5" strokeWidth={2} />
            Export JSON
          </button>
          <button
            onClick={() => exportCsv(selectedRows)}
            className="inline-flex items-center gap-1.5 border border-border-soft bg-surface px-2 py-0.5 font-sans text-sm font-medium text-text-muted hover:text-text hover:border-border cursor-pointer transition-none"
            style={{ borderRadius: "var(--radius-button)" }}
          >
            <Download className="h-3.5 w-3.5" strokeWidth={2} />
            Export CSV
          </button>
          <button
            onClick={() => exportJson(links)}
            title="Export all visible bridges (ignoring selection)"
            className="font-serif text-sm italic text-text-dim hover:text-text-muted cursor-pointer transition-none border-b border-transparent hover:border-text-muted"
          >
            Export all visible
          </button>
          <button
            onClick={clearSelection}
            className="ml-auto font-sans text-sm font-medium text-text-muted hover:text-text border-b border-transparent hover:border-text cursor-pointer transition-none"
          >
            Clear
          </button>
        </div>
      )}

      {/* Table — height-capped so the preview pane can show below it. */}
      <div
        className={cn(
          "overflow-y-auto border border-border-soft bg-surface",
          selected ? "max-h-[40vh]" : "flex-1 min-h-0",
        )}
      >
        <BridgeTable
          links={links}
          loading={loading}
          selected={selected}
          onSelect={setSelected}
          onShowConfHelp={() => setShowConfHelp(true)}
          selectedIds={selectedIds}
          onToggleRow={toggleRow}
          onToggleAll={toggleAll}
        />
      </div>

      {selected && (
        <ConnectionPreview
          link={selected}
          excerpt={excerpt}
          corpus={selectedCorpus}
          onClose={() => setSelected(null)}
          onOpenFullPanel={() =>
            openEntity({ kind: "bridge", corpusId, link: selected })
          }
        />
      )}

      <footer className="font-sans text-xs text-text-dim shrink-0">
        <span className="font-mono tabular-nums">{links.length}</span> links
        {selected && " · 1 selected"}
      </footer>

      {showConfHelp && (
        <ConfidenceHelpModal onClose={() => setShowConfHelp(false)} />
      )}
    </div>
  );
}

// ─── KIND SUMMARY HERO ─────────────────────────────────────────────────────

function KindSummary({
  grouped,
  total,
  compact,
  activeKind,
  onPick,
}: {
  grouped: Map<string, number>;
  total: number;
  compact: boolean;
  activeKind: string | null;
  onPick: (k: string) => void;
}) {
  const rows = useMemo(() => {
    return Array.from(grouped.entries())
      .map(([k, count]) => ({ kind: k, count }))
      .sort((a, b) => b.count - a.count);
  }, [grouped]);

  if (rows.length === 0) return null;

  if (compact) {
    // 32px ribbon when a row is selected.
    return (
      <div className="flex items-center gap-2 border border-border-soft bg-surface px-2 py-1 shrink-0 overflow-x-auto">
        <span className="font-mono text-[0.6875rem] font-semibold uppercase tracking-[0.05em] text-text-dim shrink-0">
          Surface
        </span>
        {rows.map(({ kind, count }) => {
          const active = kind === activeKind;
          return (
            <button
              key={kind}
              onClick={() => onPick(kind)}
              className={cn(
                "inline-flex items-center gap-1 border px-2 py-0.5 font-mono text-[0.6875rem] font-semibold uppercase tracking-[0.05em] cursor-pointer transition-none shrink-0",
                active
                  ? "border-accent bg-surface-overlay text-text"
                  : "border-border-soft bg-surface text-text-muted hover:border-border hover:text-text",
              )}
              style={{ borderRadius: "var(--radius-pill)" }}
            >
              <span>{kind}</span>
              <span className="opacity-70 tabular-nums">{count}</span>
            </button>
          );
        })}
      </div>
    );
  }

  // Full hero strip — proportional blocks.
  return (
    <div className="border border-border-soft bg-surface shrink-0">
      <div className="flex items-baseline justify-between border-b border-border-soft bg-surface-overlay px-3 py-2">
        <h3 className="font-serif text-base font-bold text-text">
          Surface
        </h3>
        <span className="font-mono text-xs tabular-nums text-text-dim">
          {total} total
        </span>
      </div>
      <div className="flex items-stretch h-16 -mx-[1px]">
        {rows.map(({ kind, count }) => {
          const pct = total > 0 ? (count / total) * 100 : 0;
          const active = kind === activeKind;
          // Ensure each block has a minimum width so labels can fit.
          const width = `max(7%, ${pct}%)`;
          return (
            <button
              key={kind}
              onClick={() => onPick(kind)}
              title={`${kind} · ${count} (${pct.toFixed(1)}%)`}
              className={cn(
                "flex flex-col items-start justify-center border border-border-soft px-2 cursor-pointer transition-none -ml-[1px] first:ml-0 min-w-0",
                active
                  ? "border-accent bg-surface-overlay text-text z-10 relative"
                  : "bg-surface text-text-muted hover:text-text hover:border-border",
              )}
              style={{ width, flexBasis: width }}
            >
              <span className="font-mono text-[0.625rem] font-semibold uppercase tracking-[0.05em] text-text-dim truncate w-full">
                {kind}
              </span>
              <span className="font-mono text-base font-semibold tabular-nums leading-none mt-0.5 text-text">
                {count}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

// ─── TABLE ─────────────────────────────────────────────────────────────────

function BridgeTable({
  links,
  loading,
  selected,
  onSelect,
  onShowConfHelp,
  selectedIds,
  onToggleRow,
  onToggleAll,
}: {
  links: BridgeLink[];
  loading: boolean;
  selected: BridgeLink | null;
  onSelect: (l: BridgeLink) => void;
  onShowConfHelp: () => void;
  selectedIds: Set<string>;
  onToggleRow: (l: BridgeLink) => void;
  onToggleAll: () => void;
}) {
  // Master-checkbox state. Indeterminate = some-but-not-all.
  const allChecked = links.length > 0 && selectedIds.size === links.length;
  const someChecked = selectedIds.size > 0 && !allChecked;

  return (
    <>
      {/* Wide-window header (>= 1280px). Adds a 28px checkbox column. */}
      <div className="hidden @min-[1280px]/page:grid grid-cols-[28px_140px_1fr_auto_1fr_60px_70px_28px] gap-0 border-b border-border-soft bg-surface-overlay sticky top-0 z-10">
        <div className="flex items-center justify-center px-1 py-1.5">
          <input
            type="checkbox"
            aria-label="Select all bridges"
            checked={allChecked}
            ref={(el) => {
              if (el) el.indeterminate = someChecked;
            }}
            onChange={onToggleAll}
            className="h-3.5 w-3.5 cursor-pointer accent-accent"
          />
        </div>
        <HeaderCell>Kind</HeaderCell>
        <HeaderCell>Export</HeaderCell>
        <HeaderCell>→</HeaderCell>
        <HeaderCell>Import</HeaderCell>
        <HeaderCell align="right">Lang</HeaderCell>
        <div className="flex items-center justify-end gap-1 px-2 py-2">
          <span className="font-mono text-[0.6875rem] font-semibold uppercase tracking-[0.05em] text-text-dim">
            Conf
          </span>
          <button
            onClick={onShowConfHelp}
            aria-label="Confidence help"
            title="What does confidence mean?"
            className="grid h-4 w-4 place-items-center border border-border-soft bg-surface font-serif text-[0.6875rem] text-text-dim hover:text-text hover:border-border cursor-pointer transition-none"
            style={{ borderRadius: "var(--radius-button)" }}
          >
            ?
          </button>
        </div>
        <div />
      </div>
      <div className="grid @min-[1280px]/page:hidden grid-cols-[28px_120px_1fr_auto_1fr_24px] gap-0 border-b border-border-soft bg-surface-overlay sticky top-0 z-10">
        <div className="flex items-center justify-center px-1 py-1.5">
          <input
            type="checkbox"
            aria-label="Select all bridges"
            checked={allChecked}
            ref={(el) => {
              if (el) el.indeterminate = someChecked;
            }}
            onChange={onToggleAll}
            className="h-3.5 w-3.5 cursor-pointer accent-accent"
          />
        </div>
        <HeaderCell>Kind</HeaderCell>
        <HeaderCell>Export</HeaderCell>
        <HeaderCell>→</HeaderCell>
        <HeaderCell>Import</HeaderCell>
        <div />
      </div>

      {links.length === 0 ? (
        <div className="px-3 py-8 font-serif text-base italic text-text-dim text-center">
          {loading ? "Querying…" : "No bridges found."}
        </div>
      ) : (
        links.map((l, i) => {
          const isSel = selected === l;
          const isChecked = selectedIds.has(bridgeKey(l));
          const cls = cn(
            "relative w-full text-left transition-none border-b border-border-soft",
            isSel
              ? "bg-surface-overlay text-text"
              : isChecked
                ? "bg-surface-overlay/60"
                : "hover:bg-surface-overlay",
          );
          return (
            // Use a div + role=button instead of <button> so a real <input>
            // checkbox can live inside without nesting interactive controls.
            <div
              key={`${l.kind}-${i}`}
              role="button"
              tabIndex={0}
              onClick={() => onSelect(l)}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onSelect(l);
                }
              }}
              className={cn(cls, "cursor-pointer")}
              title={`${l.export_language}/${l.import_language} · ${(l.confidence * 100).toFixed(0)}% confidence`}
            >
              {isSel && (
                <span className="absolute left-0 top-0 bottom-0 w-[3px] bg-accent" />
              )}

              {/* Wide grid — full columns + leading checkbox */}
              <div className="hidden @min-[1280px]/page:grid grid-cols-[28px_140px_1fr_auto_1fr_60px_70px_28px] gap-0">
                <div
                  className="flex items-center justify-center px-1 py-1.5"
                  onClick={(e) => e.stopPropagation()}
                >
                  <input
                    type="checkbox"
                    aria-label={`Select bridge ${l.export_symbol || l.export_binding_key} → ${l.import_symbol || l.import_binding_key}`}
                    checked={isChecked}
                    onChange={() => onToggleRow(l)}
                    className="h-3.5 w-3.5 cursor-pointer accent-accent"
                  />
                </div>
                <Cell>
                  <span className="font-mono text-xs font-semibold uppercase tracking-[0.05em] text-text-muted">
                    {l.kind}
                  </span>
                </Cell>
                <Cell>
                  <span className="font-mono text-sm truncate">
                    {l.export_symbol || l.export_binding_key}
                  </span>
                </Cell>
                <Cell>
                  <span className="font-mono text-text-dim">→</span>
                </Cell>
                <Cell>
                  <span className="font-mono text-sm truncate">
                    {l.import_symbol || l.import_binding_key}
                  </span>
                </Cell>
                <Cell align="right">
                  <span className="font-mono text-[0.6875rem] uppercase tracking-[0.05em] text-text-dim">
                    {l.export_language}/{l.import_language}
                  </span>
                </Cell>
                <Cell align="right">
                  <span className="font-mono text-xs tabular-nums">
                    {(l.confidence * 100).toFixed(0)}%
                  </span>
                </Cell>
                <div className="flex items-center justify-center pr-2">
                  {isSel && (
                    <ChevronRight
                      className="h-3 w-3"
                      strokeWidth={2}
                    />
                  )}
                </div>
              </div>

              {/* Narrow grid — leading checkbox + LANG inlined as chip in EXPORT */}
              <div className="grid @min-[1280px]/page:hidden grid-cols-[28px_120px_1fr_auto_1fr_24px] gap-0">
                <div
                  className="flex items-center justify-center px-1 py-1.5"
                  onClick={(e) => e.stopPropagation()}
                >
                  <input
                    type="checkbox"
                    aria-label={`Select bridge ${l.export_symbol || l.export_binding_key} → ${l.import_symbol || l.import_binding_key}`}
                    checked={isChecked}
                    onChange={() => onToggleRow(l)}
                    className="h-3.5 w-3.5 cursor-pointer accent-accent"
                  />
                </div>
                <Cell>
                  <span className="font-mono text-xs font-semibold uppercase tracking-[0.05em] text-text-muted">
                    {l.kind}
                  </span>
                </Cell>
                <Cell>
                  <span className="inline-flex items-center gap-1 min-w-0">
                    <span className="border border-border-soft px-1 font-mono text-[0.625rem] uppercase tracking-[0.05em] text-text-dim shrink-0">
                      {l.export_language}
                    </span>
                    <span className="font-mono text-sm truncate">
                      {l.export_symbol || l.export_binding_key}
                    </span>
                  </span>
                </Cell>
                <Cell>
                  <span className="font-mono text-text-dim">→</span>
                </Cell>
                <Cell>
                  <span className="inline-flex items-center gap-1 min-w-0">
                    <span className="border border-border-soft px-1 font-mono text-[0.625rem] uppercase tracking-[0.05em] text-text-dim shrink-0">
                      {l.import_language}
                    </span>
                    <span className="font-mono text-sm truncate">
                      {l.import_symbol || l.import_binding_key}
                    </span>
                  </span>
                </Cell>
                <div className="flex items-center justify-center pr-2">
                  {isSel && (
                    <ChevronRight className="h-3 w-3" strokeWidth={2} />
                  )}
                </div>
              </div>
            </div>
          );
        })
      )}
    </>
  );
}

// ─── CONNECTION PREVIEW ────────────────────────────────────────────────────

function ConnectionPreview({
  link,
  excerpt,
  corpus,
  onClose,
  onOpenFullPanel,
}: {
  link: BridgeLink;
  excerpt: ExcerptState;
  corpus: CorpusInfo | null;
  onClose: () => void;
  onOpenFullPanel?: () => void;
}) {
  return (
    <section className="flex-1 min-h-0 flex flex-col gap-2 border border-border-soft bg-surface border-l-[6px] border-l-accent">
      <div className="flex items-center justify-between border-b-2 border-border bg-surface-overlay px-3 py-2 shrink-0">
        <div className="flex items-center gap-2">
          <span className="font-mono text-xs font-bold uppercase tracking-[0.05em] text-accent">
            CONNECTION
          </span>
          <span className="font-mono text-xs uppercase tracking-[0.05em] text-text-dim">
            {link.kind}
          </span>
        </div>
        <div className="flex items-center gap-2">
          {onOpenFullPanel && (
            <button
              onClick={onOpenFullPanel}
              className="inline-flex items-center gap-1 border-2 border-border bg-surface px-2 py-0.5 font-mono text-xs font-bold uppercase tracking-[0.05em] text-text hover:bg-surface-overlay hover:text-text cursor-pointer transition-none"
              title="Open full panel"
            >
              <ExternalLink className="h-3 w-3" strokeWidth={2.5} />
              Full panel
            </button>
          )}
          <button
            onClick={onClose}
            className="grid h-6 w-6 place-items-center border-2 border-border hover:bg-surface-overlay hover:text-text cursor-pointer transition-none"
            aria-label="Close preview"
          >
            <X className="h-3 w-3" strokeWidth={2.5} />
          </button>
        </div>
      </div>

      <div className="flex-1 min-h-0 flex items-stretch gap-0 px-3 pb-3">
        <CodePane
          title="EXPORT"
          language={link.export_language}
          file={link.export_file}
          line={link.export_line}
          symbol={link.export_symbol}
          binding={link.export_binding_key}
          source={excerpt.exportSrc}
          startLine={excerpt.exportStart}
          loading={excerpt.loading}
          corpus={corpus}
        />

        <Connector />

        <CodePane
          title="IMPORT"
          language={link.import_language}
          file={link.import_file}
          line={link.import_line}
          symbol={link.import_symbol}
          binding={link.import_binding_key}
          source={excerpt.importSrc}
          startLine={excerpt.importStart}
          loading={excerpt.loading}
          corpus={corpus}
        />
      </div>

      <div className="border-t-2 border-border bg-surface-overlay px-3 py-1.5 flex items-center justify-between shrink-0">
        <span className="font-mono text-xs uppercase tracking-[0.05em] text-text">
          {link.kind} · {link.export_symbol || link.export_binding_key}
        </span>
        <span className="font-mono text-xs tabular-nums text-text-dim">
          CONFIDENCE {(link.confidence * 100).toFixed(0)}%
        </span>
      </div>
    </section>
  );
}

function CodePane({
  title,
  language,
  file,
  line,
  symbol,
  binding,
  source,
  startLine,
  loading,
  corpus,
}: {
  title: string;
  language: string;
  file: string;
  line: number;
  symbol: string;
  binding: string;
  source: string | null;
  startLine: number | null;
  loading: boolean;
  corpus: CorpusInfo | null;
}) {
  const tail = corpusRelative(file, corpus);

  // Split source into lines with explicit numbers.
  const numbered = useMemo(() => {
    if (loading || !source) return null;
    const lines = source.split("\n");
    const start = startLine ?? 1;
    const focus = line;
    return lines.map((content, idx) => ({
      n: start + idx,
      content,
      focus: start + idx === focus,
    }));
  }, [source, startLine, line, loading]);

  return (
    <div className="flex-1 min-w-0 flex flex-col border border-border-soft bg-surface">
      <div className="flex items-center justify-between border-b-2 border-border bg-surface-overlay px-2 py-1.5 shrink-0">
        <div className="flex items-center gap-2 min-w-0">
          <span className="font-mono text-xs font-bold uppercase tracking-[0.05em] text-accent shrink-0">
            {title}
          </span>
          <span className="font-mono text-xs uppercase tracking-[0.05em] text-text-dim shrink-0">
            {language}
          </span>
        </div>
        <span className="font-mono text-xs text-text-dim truncate ml-2">
          {tail}:{line}
        </span>
      </div>

      <div className="border-b-2 border-border px-2 py-1 shrink-0">
        <span className="font-mono text-xs font-bold text-text break-words">
          {symbol || binding}
        </span>
      </div>

      <div className="flex-1 min-h-0 overflow-auto bg-surface-sunken font-mono text-[0.6875rem] leading-relaxed">
        {loading || !numbered ? (
          <div className="px-3 py-2 text-text">
            LOADING<span className="ministr-blink">_</span>
          </div>
        ) : (
          <table className="w-full border-collapse">
            <tbody>
              {numbered.map(({ n, content, focus }) => (
                <tr
                  key={n}
                  className={cn(
                    focus && "bg-surface-overlay",
                  )}
                >
                  <td className="select-none border-r-2 border-border px-2 text-right text-text-dim tabular-nums w-10 align-top">
                    {n}
                  </td>
                  <td className="px-3 whitespace-pre text-text">
                    {content || " "}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}

function Connector() {
  return (
    <div
      className="flex flex-col items-center justify-center w-16 shrink-0 self-stretch"
      aria-hidden="true"
    >
      <div className="flex items-center gap-1">
        <span className="block h-3 w-2 bg-accent" />
        <span className="block h-3 w-2 bg-accent" />
        <span className="block h-3 w-2 bg-accent" />
        <span
          className="block"
          style={{
            width: 0,
            height: 0,
            borderTop: "8px solid transparent",
            borderBottom: "8px solid transparent",
            borderLeft: "10px solid var(--color-accent)",
          }}
        />
      </div>
    </div>
  );
}

// ─── CONFIDENCE HELP MODAL ─────────────────────────────────────────────────

function ConfidenceHelpModal({ onClose }: { onClose: () => void }) {
  return (
    <div
      className="fixed inset-0 z-[1000] flex items-start justify-center bg-black/40 px-6"
      style={{ paddingTop: "20vh" }}
      role="dialog"
      aria-modal="true"
      aria-label="Confidence help"
      onClick={onClose}
    >
      <div
        className="w-full max-w-md border border-border-soft bg-surface shadow-[6px_6px_0_0_var(--shadow-color)]"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b-2 border-border bg-surface-overlay px-3 py-2">
          <span className="font-mono text-[0.6875rem] font-bold uppercase tracking-[0.05em] text-text">
            CONFIDENCE
          </span>
          <button
            onClick={onClose}
            aria-label="Close"
            className="grid h-6 w-6 place-items-center border-2 border-border hover:bg-surface-overlay hover:text-text cursor-pointer transition-none"
          >
            <X className="h-3 w-3" strokeWidth={2.5} />
          </button>
        </div>
        <div className="p-4 font-mono text-xs text-text leading-relaxed">
          {CONFIDENCE_HELP}
        </div>
      </div>
    </div>
  );
}

// ─── FILTERS / TABLE PRIMITIVES ────────────────────────────────────────────

function FilterPill({
  label,
  count,
  active,
  onClick,
}: {
  label: string;
  count?: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "border px-2 py-0.5 text-[0.6875rem] font-mono font-semibold uppercase tracking-[0.05em] cursor-pointer transition-none",
        active
          ? "border-accent bg-surface-overlay text-text"
          : "border-border-soft bg-surface text-text-muted hover:border-border hover:text-text",
      )}
      style={{ borderRadius: "var(--radius-pill)" }}
    >
      {label}
      {typeof count === "number" && count > 0 && (
        <span className="ml-1 tabular-nums opacity-70">{count}</span>
      )}
    </button>
  );
}

function HeaderCell({
  children,
  align,
}: {
  children: React.ReactNode;
  align?: "right";
}) {
  return (
    <div
      className={cn(
        "font-mono text-[0.6875rem] font-semibold uppercase tracking-[0.05em] text-text-dim px-2 py-2",
        align === "right" && "text-right",
      )}
    >
      {children}
    </div>
  );
}

function Cell({
  children,
  align,
}: {
  children: React.ReactNode;
  align?: "right";
}) {
  return (
    <div
      className={cn(
        "px-2 py-1.5 flex items-center min-w-0",
        align === "right" && "justify-end",
      )}
    >
      {children}
    </div>
  );
}
