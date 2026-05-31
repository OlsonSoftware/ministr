/**
 * ReferencesPanel — callers / importers / implementors of a symbol.
 *
 * Single responsibility: fetch `symbol_references` and present them grouped by
 * file, with ref-kind shown. Clicking a reference asks the parent to jump
 * (the parent resolves the caller's line via `search_symbols`, since the
 * reference edge itself carries names + files, not line numbers).
 */
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowLeft } from "lucide-react";
import type { SymbolRef } from "../../lib/types";

interface Props {
  corpusId: string;
  symbolId: string;
  symbolName: string;
  onBack: () => void;
  onJump: (ref: SymbolRef) => void;
}

interface FileGroup {
  file: string;
  refs: SymbolRef[];
}

export function ReferencesPanel({
  corpusId,
  symbolId,
  symbolName,
  onBack,
  onJump,
}: Props) {
  const [refs, setRefs] = useState<SymbolRef[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setRefs([]);
    invoke<SymbolRef[]>("symbol_references", { corpusId, symbolId })
      .then((r) => {
        if (!cancelled) setRefs(r);
      })
      .catch(() => {
        if (!cancelled) setRefs([]);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [corpusId, symbolId]);

  const groups = useMemo<FileGroup[]>(() => {
    const byFile = new Map<string, SymbolRef[]>();
    for (const r of refs) {
      const list = byFile.get(r.from_file) ?? [];
      list.push(r);
      byFile.set(r.from_file, list);
    }
    return [...byFile.entries()]
      .map(([file, list]) => ({ file, refs: list }))
      .sort((a, b) => a.file.localeCompare(b.file));
  }, [refs]);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex items-center justify-between gap-2 border-b border-border-soft bg-surface-overlay px-3 py-2">
        <div className="flex min-w-0 items-center gap-2">
          <span className="font-mono text-xs font-bold uppercase tracking-[0.08em] text-accent">
            References
          </span>
          <span className="truncate font-mono text-xs font-semibold text-text">
            {symbolName}
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <span className="font-mono text-mono-mini tabular-nums text-text-dim">
            {refs.length}
          </span>
          <button
            type="button"
            onClick={onBack}
            className="inline-flex items-center gap-1 rounded-md border border-border-soft px-1.5 py-0.5 font-mono text-mono-mini font-bold uppercase tracking-[0.08em] text-text-muted hover:border-border hover:text-text cursor-pointer transition-colors duration-150 ease-out"
          >
            <ArrowLeft className="h-2.5 w-2.5" strokeWidth={2.5} />
            Peek
          </button>
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {loading ? (
          <p className="px-3 py-2 font-mono text-mono-mini text-text-dim">Loading_</p>
        ) : groups.length === 0 ? (
          <p className="px-3 py-2 font-mono text-mono-mini text-text-dim">
            No references found.
          </p>
        ) : (
          groups.map((g) => (
            <section key={g.file}>
              <div className="sticky top-0 border-b border-border-soft bg-surface px-3 py-1 font-mono text-mono-mini text-text-dim">
                <span className="truncate">{g.file}</span>
              </div>
              {g.refs.map((r, i) => (
                <button
                  key={`${r.from_name}-${i}`}
                  type="button"
                  onClick={() => onJump(r)}
                  title={`Jump to ${r.from_name} in ${r.from_file}`}
                  className="flex w-full items-center gap-2 border-b border-border-soft px-3 py-1.5 text-left font-mono text-mono-mini text-text-muted last:border-b-0 hover:bg-surface-overlay hover:text-text cursor-pointer transition-colors duration-150 ease-out"
                >
                  <span className="truncate flex-1 text-text">{r.from_name}</span>
                  <span className="shrink-0 rounded-sm border border-border-soft px-1 text-mono-micro uppercase tracking-[0.08em] opacity-70">
                    {r.ref_kind}
                  </span>
                </button>
              ))}
            </section>
          ))
        )}
      </div>
    </div>
  );
}
