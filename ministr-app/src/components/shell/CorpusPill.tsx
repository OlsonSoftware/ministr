import { useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, X } from "lucide-react";
import type { CorpusInfo } from "../../lib/types";
import { corpusLabel } from "../../lib/corpus";
import { cn } from "../../lib/utils";
import { StatusDot } from "../ui/status-dot";
import { corpusTone, isCorpusLive } from "../../lib/status";

interface Props {
  corpora: readonly CorpusInfo[];
  activeCorpus: CorpusInfo | null;
  onSelect: (id: string) => void;
}

/**
 * Persistent active-corpus pill in the TopBar. Shows the current scope and
 * opens a focused mini-palette for switching corpora.
 *
 * The picker is a tiny dropdown anchored to the pill. Type-to-filter; click
 * a row to commit; Esc closes.
 */
export function CorpusPill({ corpora, activeCorpus, onSelect }: Props) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setActive(0);
    setTimeout(() => inputRef.current?.focus(), 30);

    function onClick(e: MouseEvent) {
      if (!containerRef.current?.contains(e.target as Node)) setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    window.addEventListener("mousedown", onClick);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onClick);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return corpora;
    return corpora.filter((c) =>
      [c.id, ...c.paths, corpusLabel(c)].some((s) =>
        s.toLowerCase().includes(q),
      ),
    );
  }, [corpora, query]);

  // Clamp `active` whenever the filtered set shrinks below the current
  // index. Without this, typing a query that drops `filtered.length`
  // below `active` leaves a stale pointer; the next Enter would try to
  // index `filtered[active]` (undefined) and silently do nothing.
  useEffect(() => {
    if (active >= filtered.length) {
      setActive(filtered.length === 0 ? 0 : filtered.length - 1);
    }
  }, [filtered.length, active]);

  function commit(id: string) {
    onSelect(id);
    setOpen(false);
  }

  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => Math.min(a + 1, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(a - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const c = filtered[active];
      if (c) commit(c.id);
    }
  }

  if (corpora.length === 0) {
    return (
      <span className="inline-flex items-center gap-1.5 border border-border-soft bg-surface px-2 py-0.5 font-sans text-xs font-bold tracking-[0.05em] text-text-dim">
        No corpus
      </span>
    );
  }

  const tone = activeCorpus ? corpusTone(activeCorpus) : "muted";
  const live = activeCorpus ? isCorpusLive(activeCorpus) : false;

  return (
    <div ref={containerRef} className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        title="Switch corpus"
        className={cn(
          "inline-flex items-center gap-1.5 border border-border-soft bg-surface px-2 py-1 font-sans text-sm font-medium cursor-pointer transition-none",
          "hover:bg-surface-overlay hover:border-border",
          open && "bg-surface-overlay border-accent text-text",
        )}
        style={{ borderRadius: "var(--radius-button)" }}
      >
        <StatusDot tone={tone} pulse={live ? "live" : "off"} />
        <span className="text-[0.6875rem] font-mono uppercase tracking-[0.05em] text-text-dim">
          Corpus
        </span>
        <span className="font-mono tabular-nums text-text">
          {activeCorpus ? corpusLabel(activeCorpus) : "—"}
        </span>
        <ChevronDown className="h-3 w-3 text-text-dim" strokeWidth={2} />
      </button>

      {open && (
        <div className="absolute top-full left-0 mt-1 z-50 w-[360px] border border-border-soft bg-surface shadow-[var(--shadow-md)]">
          <div className="flex items-center gap-2 border-b border-border-soft bg-surface-overlay px-3 py-2">
            <span className="font-mono text-base font-bold text-accent">{">"}</span>
            <input
              ref={inputRef}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={onKeyDown}
              placeholder="filter corpora"
              spellCheck={false}
              autoComplete="off"
              className="flex-1 bg-transparent text-sm font-sans text-text placeholder:text-text-dim outline-none"
            />
            <button
              onClick={() => setOpen(false)}
              aria-label="Close picker"
              className="grid h-5 w-5 place-items-center border border-border-soft text-text-muted hover:text-text hover:border-border cursor-pointer transition-none"
              style={{ borderRadius: "var(--radius-button)" }}
            >
              <X className="h-2.5 w-2.5" strokeWidth={2} />
            </button>
          </div>
          <div className="max-h-72 overflow-y-auto">
            {filtered.length === 0 ? (
              <p className="px-3 py-3 font-serif text-sm italic text-text-dim">
                No matches.
              </p>
            ) : (
              filtered.map((c, i) => {
                const isActive = i === active;
                const isCurrent = activeCorpus?.id === c.id;
                return (
                  <button
                    key={c.id}
                    onClick={() => commit(c.id)}
                    onMouseEnter={() => setActive(i)}
                    className={cn(
                      "relative w-full text-left flex items-start gap-2 border-b border-border-soft last:border-b-0 px-3 py-2 cursor-pointer transition-none",
                      isActive
                        ? "bg-surface-overlay text-text"
                        : "bg-surface text-text hover:bg-surface-overlay",
                    )}
                  >
                    {isActive && (
                      <span className="absolute left-0 top-0 bottom-0 w-[3px] bg-accent" />
                    )}
                    <StatusDot
                      tone={corpusTone(c)}
                      pulse={isCorpusLive(c) ? "live" : "off"}
                      className="mt-1"
                    />
                    <div className="min-w-0 flex-1">
                      <div className="font-mono text-sm font-semibold truncate">
                        {corpusLabel(c)}
                        {isCurrent && (
                          <span className="ml-2 font-mono text-[0.625rem] uppercase tracking-[0.05em] text-text-dim">
                            active
                          </span>
                        )}
                      </div>
                      <div className="font-mono text-xs text-text-dim truncate">
                        {c.paths[0]}
                      </div>
                    </div>
                    <span className="font-mono text-xs tabular-nums text-text-dim shrink-0 mt-0.5">
                      {c.sections_count.toLocaleString()}
                    </span>
                  </button>
                );
              })
            )}
          </div>
        </div>
      )}
    </div>
  );
}
