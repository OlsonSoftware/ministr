/**
 * DeadCodeMap — the Explore "Unused" lens: symbols the reference graph says
 * NOTHING calls, implements, or imports, and that don't look like entry points.
 *
 * This answers the recurring "what can I safely delete?" question with the same
 * graph the AI uses — grouped by file, ranked by how much code each candidate
 * would remove, click-to-inspect (the shared EntityPanel symbol view, so you
 * confirm the zero-reference claim before deleting) and click-to-open (the code
 * lens at the symbol). Candidates, not a verdict: an entry point reachable only
 * by reflection or a macro still surfaces here, so the inspector is one click
 * away. Built fresh from the v4 tokens/atoms.
 *
 * Pure `DeadCodeMap` renders from props (Storybook); `DeadCodeMapConnector`
 * wires the `dead_code` invoke + the shared inspector.
 */
import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FileCode2, Sparkles, Trash2 } from "@/components/ui/icons";

import type { DeadSymbol, SymbolInfo } from "../../lib/types";
import { cn } from "../../lib/utils";
import { useEntityPanel } from "../../hooks/useEntityPanel";
import { useCachedQuery } from "../../hooks/useCachedQuery";
import { useArrowKeyListNav } from "../../hooks/useArrowKeyListNav";
import { LensHeader, LensLoading, LensEmpty, LensRerunButton } from "../ui/lens-frame";

function fileTail(path: string): string {
  return path.replace(/\\/g, "/").split("/").slice(-2).join("/");
}

function baseName(path: string): string {
  return path.replace(/\\/g, "/").split("/").pop() ?? path;
}

export interface DeadCodeMapProps {
  symbols: DeadSymbol[];
  loading?: boolean;
  /** Re-run the dead-code analysis (a snapshot — re-run after editing). */
  onRefresh?: () => void;
  refreshing?: boolean;
  /** Inspect a candidate in the shared EntityPanel (confirm before deleting). */
  onInspect: (sym: DeadSymbol) => void;
  /** Open the candidate's file in the code lens, focused on its line. */
  onOpenFile: (path: string, line: number) => void;
}

export function DeadCodeMap({
  symbols = [],
  loading = false,
  onRefresh,
  refreshing = false,
  onInspect,
  onOpenFile,
}: DeadCodeMapProps) {
  const [kindFilter, setKindFilter] = useState<string | null>(null);
  const listRef = useArrowKeyListNav<HTMLDivElement>();

  const kinds = useMemo(() => {
    const m = new Map<string, number>();
    for (const s of symbols) m.set(s.kind, (m.get(s.kind) ?? 0) + 1);
    return [...m.entries()].sort((a, b) => b[1] - a[1]);
  }, [symbols]);

  const deadLines = useMemo(
    () => symbols.reduce((a, s) => a + s.lines, 0),
    [symbols],
  );

  const filtered = useMemo(
    () => (kindFilter ? symbols.filter((s) => s.kind === kindFilter) : symbols),
    [symbols, kindFilter],
  );

  // Group by file, files with the most reclaimable code first.
  const groups = useMemo(() => {
    const byFile = new Map<string, DeadSymbol[]>();
    for (const s of filtered) {
      const arr = byFile.get(s.file);
      if (arr) arr.push(s);
      else byFile.set(s.file, [s]);
    }
    for (const arr of byFile.values()) arr.sort((a, b) => b.lines - a.lines);
    return [...byFile.entries()].sort(
      (a, b) =>
        b[1].reduce((x, s) => x + s.lines, 0) -
        a[1].reduce((x, s) => x + s.lines, 0),
    );
  }, [filtered]);

  if (loading) {
    return <LensLoading label="Tracing the reference graph" />;
  }

  if (symbols.length === 0) {
    return (
      <LensEmpty
        icon={Sparkles}
        accent
        title="No dead code"
        hint="Every indexed symbol is referenced (or looks like an entry point). Nothing to prune — the reference graph is clean."
        action={
          onRefresh ? (
            <LensRerunButton onRefresh={onRefresh} refreshing={refreshing} />
          ) : undefined
        }
      />
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* ── Glance header + kind filters (shared lens-chrome). ─────────── */}
      <LensHeader
        icon={Trash2}
        title="Unused candidates"
        tone="warning"
        glance={
          <>
            <span className="tabular-nums font-semibold text-text">
              {symbols.length}
            </span>{" "}
            symbols ·{" "}
            <span className="tabular-nums font-semibold text-text">
              {deadLines.toLocaleString()}
            </span>{" "}
            reclaimable lines ·{" "}
            <span className="tabular-nums font-semibold text-text">
              {groups.length}
            </span>{" "}
            files
          </>
        }
        hint="Zero references in the graph — candidates, not a verdict. Inspect to confirm before deleting."
        onRefresh={onRefresh}
        refreshing={refreshing}
      >
        <div className="flex flex-wrap gap-1.5">
          <KindChip
            label="All"
            count={symbols.length}
            active={kindFilter === null}
            onClick={() => setKindFilter(null)}
          />
          {kinds.map(([k, n]) => (
            <KindChip
              key={k}
              label={k}
              count={n}
              active={kindFilter === k}
              onClick={() => setKindFilter(kindFilter === k ? null : k)}
            />
          ))}
        </div>
      </LensHeader>

      {/* ── Candidates, grouped by file. ───────────────────────────────── */}
      <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto">
        {groups.map(([file, group]) => {
          const reclaim = group.reduce((a, s) => a + s.lines, 0);
          return (
            <section
              key={file}
              className="border-b border-border-soft last:border-b-0"
            >
              <header className="sticky top-0 z-10 flex items-center gap-2 border-b border-border-soft bg-surface-overlay/95 px-4 py-1.5 backdrop-blur">
                <FileCode2
                  className="h-3.5 w-3.5 text-text-dim"
                  strokeWidth={2}
                />
                <button
                  type="button"
                  onClick={() => onOpenFile(file, group[0]?.line ?? 1)}
                  title={`Open ${file}`}
                  className="truncate font-mono text-mono-micro font-bold uppercase tracking-[0.06em] text-text hover:text-accent cursor-pointer transition-colors duration-150"
                >
                  {baseName(file)}
                </button>
                <span className="truncate font-mono text-mono-micro text-text-dim">
                  {fileTail(file)}
                </span>
                <span className="ml-auto shrink-0 font-mono text-mono-micro tabular-nums text-text-dim">
                  {reclaim} ln · {group.length}
                </span>
              </header>
              <div className="divide-y divide-border-soft/60">
                {group.map((s) => (
                  <DeadRow
                    key={s.symbol_id}
                    sym={s}
                    onInspect={() => onInspect(s)}
                    onOpenFile={onOpenFile}
                  />
                ))}
              </div>
            </section>
          );
        })}
      </div>
    </div>
  );
}

function DeadRow({
  sym,
  onInspect,
  onOpenFile,
}: {
  sym: DeadSymbol;
  onInspect: () => void;
  onOpenFile: (path: string, line: number) => void;
}) {
  return (
    // Row is a plain container (NOT a button) so the inner "open :line" button
    // isn't a nested interactive control (a11y: nested-interactive). The
    // row-inspect action is its own sibling button that fills the row.
    <div className="group flex items-center gap-2.5 px-4 py-2 hover:bg-surface-overlay transition-colors duration-150 ease-out">
      <button
        type="button"
        data-roving-item
        onClick={onInspect}
        title="Inspect — confirm it's truly unreferenced before deleting"
        className="flex min-w-0 flex-1 items-center gap-2.5 text-left cursor-pointer"
      >
        <span className="shrink-0 rounded border border-border-soft bg-surface px-1 font-mono text-mono-micro font-semibold uppercase tracking-[0.06em] text-text-dim">
          {sym.kind}
        </span>
        <span className="truncate font-mono text-xs font-semibold text-text">
          {sym.name}
        </span>
        <span className="shrink-0 font-mono text-mono-micro uppercase tracking-[0.06em] text-text-dim">
          {sym.visibility}
        </span>
      </button>
      <button
        type="button"
        onClick={() => onOpenFile(sym.file, sym.line)}
        title={`Open ${sym.file}:${sym.line}`}
        className="shrink-0 font-mono text-mono-micro text-text-dim hover:text-accent cursor-pointer transition-colors duration-150"
      >
        :{sym.line}
      </button>
      <span className="shrink-0 rounded-full border border-warning/40 px-1.5 font-mono text-mono-micro tabular-nums text-warning">
        {sym.lines} ln
      </span>
    </div>
  );
}

function KindChip({
  label,
  count,
  active,
  onClick,
}: {
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "inline-flex items-center gap-1 rounded-md border px-2 py-0.5 font-mono text-mono-mini uppercase tracking-[0.06em] cursor-pointer transition-colors duration-150 ease-out",
        active
          ? "border-accent bg-surface-overlay text-text"
          : "border-border-soft bg-surface text-text-muted hover:border-border hover:text-text",
      )}
    >
      <span className="font-semibold">{label}</span>
      <span className="tabular-nums">{count}</span>
    </button>
  );
}

// ───────────────────────────────────────────────────────────────────────────
// Connector — fetches dead-code candidates + wires the shared inspector.

function deadToSymbolInfo(d: DeadSymbol): SymbolInfo {
  return {
    id: d.symbol_id,
    name: d.name,
    kind: d.kind,
    file_path: d.file,
    visibility: d.visibility,
    signature: `${d.kind} ${d.name}`,
    doc_comment: null,
    module_path: "",
  };
}

export function DeadCodeMapConnector({
  corpusId,
  onOpenFile,
}: {
  corpusId: string;
  onOpenFile: (path: string, line: number) => void;
}) {
  const { openEntity } = useEntityPanel();
  const { data, loading, refreshing, refresh } = useCachedQuery<DeadSymbol[]>(
    corpusId,
    "dead_code",
    () =>
      invoke<DeadSymbol[]>("dead_code", {
        corpusId,
        kind: null,
        module: null,
        minLines: null,
        limit: 500,
      }),
    [],
  );

  return (
    <DeadCodeMap
      symbols={data}
      loading={loading}
      onRefresh={refresh}
      refreshing={refreshing}
      onInspect={(d) =>
        openEntity({ kind: "symbol", corpusId, symbol: deadToSymbolInfo(d) })
      }
      onOpenFile={onOpenFile}
    />
  );
}
