/**
 * SymbolPalette — ⌘K symbol-jump for the Code surface.
 *
 * Single responsibility: resolve a typed query to a symbol via
 * `search_symbols` and report the pick. This is also the v1 fallback for
 * "unresolved tokens" — anything not clickable in the viewer can be reached
 * by name here.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AnimatePresence, motion } from "motion/react";
import { cn } from "../../lib/utils";
import { fade } from "../../lib/motion";
import type { SymbolInfo } from "../../lib/types";

interface Props {
  open: boolean;
  corpusId: string;
  onClose: () => void;
  onPick: (symbol: SymbolInfo) => void;
}

export function SymbolPalette({ open, corpusId, onClose, onPick }: Props) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SymbolInfo[]>([]);
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);

  // Reset + focus on open.
  useEffect(() => {
    if (!open) return;
    setQuery("");
    setResults([]);
    setActive(0);
    const id = window.setTimeout(() => inputRef.current?.focus(), 0);
    return () => window.clearTimeout(id);
  }, [open]);

  // Debounced search.
  useEffect(() => {
    if (!open || !corpusId) return;
    let cancelled = false;
    const id = window.setTimeout(() => {
      invoke<SymbolInfo[]>("search_symbols", {
        corpusId,
        query: query.trim(),
        kind: null,
        filePath: null,
      })
        .then((r) => {
          if (!cancelled) {
            setResults(r.slice(0, 50));
            setActive(0);
          }
        })
        .catch(() => {
          if (!cancelled) setResults([]);
        });
    }, 120);
    return () => {
      cancelled = true;
      window.clearTimeout(id);
    };
  }, [open, corpusId, query]);

  const onKeyDown = useMemo(
    () => (e: React.KeyboardEvent) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActive((a) => Math.min(results.length - 1, a + 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setActive((a) => Math.max(0, a - 1));
      } else if (e.key === "Enter") {
        e.preventDefault();
        const pick = results[active];
        if (pick) onPick(pick);
      } else if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    },
    [results, active, onPick, onClose],
  );

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          variants={fade}
          initial="initial"
          animate="animate"
          exit="exit"
          className="absolute inset-0 z-40 flex items-start justify-center bg-bg/60 pt-[12vh]"
          onClick={onClose}
        >
          <div
            className="w-[min(560px,90%)] overflow-hidden rounded-lg border border-border bg-surface shadow-[var(--glow-soft)]"
            onClick={(e) => e.stopPropagation()}
          >
            <input
              ref={inputRef}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={onKeyDown}
              placeholder="Jump to symbol…"
              className="h-11 w-full border-b border-border-soft bg-surface px-3 font-mono text-sm text-text placeholder:text-text-dim focus:outline-none"
            />
            <div className="max-h-[44vh] overflow-y-auto">
              {results.length === 0 ? (
                <p className="px-3 py-3 font-mono text-mono-mini text-text-dim">
                  {query ? "No matches." : "Type to search symbols."}
                </p>
              ) : (
                results.map((s, i) => (
                  <button
                    key={s.id}
                    type="button"
                    onClick={() => onPick(s)}
                    onMouseMove={() => setActive(i)}
                    className={cn(
                      "flex w-full items-center gap-2 px-3 py-1.5 text-left font-mono text-xs cursor-pointer transition-colors duration-150 ease-out",
                      i === active
                        ? "bg-surface-overlay text-text"
                        : "text-text-muted hover:bg-surface-overlay hover:text-text",
                    )}
                  >
                    <span className="w-12 shrink-0 text-mono-mini uppercase tracking-[0.08em] text-text-dim">
                      {s.kind}
                    </span>
                    <span className="shrink-0 font-semibold">{s.name}</span>
                    <span className="ml-auto truncate text-mono-mini text-text-dim">
                      {s.file_path}
                    </span>
                  </button>
                ))
              )}
            </div>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
