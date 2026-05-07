import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown, Plus } from "lucide-react";
import { cn } from "../../lib/utils";
import { corpusLabel, corpusRoot } from "../../lib/corpus";
import type { CorpusInfo } from "../../lib/types";

interface Props {
  corpora: CorpusInfo[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onAddProject: () => void;
}

/**
 * Persistent project switcher in the top bar. Replaces the old tray-menu
 * corpus quick-access and the per-page corpus dropdowns. Always visible,
 * always one click. Empty state shows an "Add project" CTA so a user with
 * no projects can recover without finding the Projects tab.
 */
export function ProjectPicker({ corpora, activeId, onSelect, onAddProject }: Props) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function onClick(e: MouseEvent) {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
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

  const active = corpora.find((c) => c.id === activeId) ?? null;

  if (corpora.length === 0) {
    return (
      <button
        type="button"
        onClick={onAddProject}
        className={cn(
          "inline-flex items-center gap-1.5 px-3 h-8 cursor-pointer transition-none",
          "border-2 border-border bg-surface text-text",
          "hover:bg-surface-overlay rounded-sm",
          "font-mono text-xs font-semibold uppercase tracking-[0.05em]",
        )}
      >
        <Plus className="h-3.5 w-3.5" strokeWidth={2.5} />
        Add project
      </button>
    );
  }

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="listbox"
        aria-expanded={open}
        className={cn(
          "inline-flex items-center gap-2 px-3 h-8 cursor-pointer transition-none",
          "border-2 border-border bg-surface text-text",
          "hover:bg-surface-overlay rounded-sm max-w-[280px]",
        )}
      >
        <span className="font-mono text-xs font-semibold uppercase tracking-[0.05em] truncate">
          {active ? corpusLabel(active) : "Select project"}
        </span>
        <ChevronDown
          className={cn("h-3.5 w-3.5 shrink-0 transition-none", open && "rotate-180")}
          strokeWidth={2.5}
        />
      </button>

      {open && (
        <div
          role="listbox"
          className={cn(
            "absolute top-full left-0 mt-1 z-50",
            "min-w-[280px] max-w-[420px]",
            "border-2 border-border bg-surface shadow-md",
          )}
        >
          <ul className="max-h-[320px] overflow-y-auto">
            {corpora.map((c) => {
              const isActive = c.id === activeId;
              const root = corpusRoot(c.paths);
              return (
                <li key={c.id}>
                  <button
                    type="button"
                    role="option"
                    aria-selected={isActive}
                    onClick={() => {
                      onSelect(c.id);
                      setOpen(false);
                    }}
                    className={cn(
                      "w-full flex items-center gap-2 px-3 py-2 text-left cursor-pointer transition-none",
                      "border-b border-border-soft last:border-b-0",
                      isActive
                        ? "bg-surface-overlay text-text"
                        : "bg-surface text-text-muted hover:bg-surface-overlay hover:text-text",
                    )}
                  >
                    <Check
                      className={cn(
                        "h-3.5 w-3.5 shrink-0",
                        isActive ? "text-accent" : "text-transparent",
                      )}
                      strokeWidth={3}
                    />
                    <div className="flex-1 min-w-0">
                      <div className="font-mono text-xs font-semibold uppercase tracking-[0.05em] truncate">
                        {corpusLabel(c)}
                      </div>
                      {root && (
                        <div className="font-mono text-mono-mini text-text-dim truncate">
                          {root}
                        </div>
                      )}
                    </div>
                  </button>
                </li>
              );
            })}
          </ul>
          <button
            type="button"
            onClick={() => {
              onAddProject();
              setOpen(false);
            }}
            className={cn(
              "w-full flex items-center gap-2 px-3 py-2 cursor-pointer transition-none",
              "border-t-2 border-border bg-surface-overlay text-text",
              "hover:bg-surface font-mono text-xs font-semibold uppercase tracking-[0.05em]",
            )}
          >
            <Plus className="h-3.5 w-3.5" strokeWidth={2.5} />
            Add project
          </button>
        </div>
      )}
    </div>
  );
}
