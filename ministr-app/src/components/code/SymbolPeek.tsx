/**
 * SymbolPeek — the inline "peek" for a clicked symbol.
 *
 * Single responsibility: fetch and present one symbol's definition
 * (`symbol_definition`) — signature, location, doc, and source context — and
 * expose the two onward actions: go to its definition, or show its references.
 */
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowUpRight, GitFork, X } from "lucide-react";
import type { SymbolDefinitionDetail } from "../../lib/types";

interface Props {
  corpusId: string;
  symbolId: string;
  symbolName: string;
  onGoToDefinition: (filePath: string, line: number) => void;
  onShowReferences: () => void;
  onClose: () => void;
}

export function SymbolPeek({
  corpusId,
  symbolId,
  symbolName,
  onGoToDefinition,
  onShowReferences,
  onClose,
}: Props) {
  const [def, setDef] = useState<SymbolDefinitionDetail | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setDef(null);
    invoke<SymbolDefinitionDetail>("symbol_definition", { corpusId, symbolId })
      .then((d) => {
        if (!cancelled) setDef(d);
      })
      .catch(() => {
        if (!cancelled) setDef(null);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [corpusId, symbolId]);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex items-center justify-between gap-2 border-b border-border-soft bg-surface-overlay px-3 py-2">
        <div className="flex min-w-0 items-center gap-2">
          <span className="font-mono text-xs font-bold uppercase tracking-[0.08em] text-accent">
            Peek
          </span>
          <span className="truncate font-mono text-xs font-semibold text-text">
            {symbolName}
          </span>
        </div>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close peek"
          className="grid h-5 w-5 shrink-0 place-items-center rounded-md border border-border-soft text-text-muted hover:border-border hover:text-text cursor-pointer transition-colors duration-150 ease-out"
        >
          <X className="h-2.5 w-2.5" strokeWidth={2} />
        </button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto p-3">
        {loading ? (
          <p className="font-mono text-mono-mini text-text-dim">Loading_</p>
        ) : !def ? (
          <p className="font-mono text-mono-mini text-text-dim">
            No definition found for this symbol.
          </p>
        ) : (
          <div className="space-y-2">
            <div className="font-mono text-xs font-bold text-text">{def.signature}</div>
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
                {def.kind}
              </span>
              {def.visibility && (
                <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
                  {def.visibility}
                </span>
              )}
            </div>
            <div className="font-mono text-mono-mini text-text-dim">
              {def.file_path}:{def.line_start}-{def.line_end}
            </div>
            {def.doc_comment && (
              <div className="border-l border-accent bg-surface-overlay px-2 py-1.5 font-mono text-mono-mini text-text-muted whitespace-pre-wrap">
                {def.doc_comment}
              </div>
            )}
            <pre className="max-h-72 overflow-auto rounded-md border border-border-soft bg-surface-sunken p-2 font-mono text-mono-mini leading-relaxed text-text whitespace-pre">
              {def.source_context}
            </pre>

            <div className="flex flex-wrap gap-2 pt-1">
              <button
                type="button"
                onClick={() => onGoToDefinition(def.file_path, def.line_start)}
                className="inline-flex items-center gap-1 rounded-md border border-border bg-surface px-2 py-1 font-mono text-mono-mini font-bold uppercase tracking-[0.08em] text-text hover:bg-surface-overlay cursor-pointer transition-colors duration-150 ease-out"
              >
                <ArrowUpRight className="h-3 w-3" strokeWidth={2.5} />
                Go to definition
              </button>
              <button
                type="button"
                onClick={onShowReferences}
                className="inline-flex items-center gap-1 rounded-md border border-border bg-surface px-2 py-1 font-mono text-mono-mini font-bold uppercase tracking-[0.08em] text-text hover:bg-surface-overlay cursor-pointer transition-colors duration-150 ease-out"
              >
                <GitFork className="h-3 w-3" strokeWidth={2.5} />
                References
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
