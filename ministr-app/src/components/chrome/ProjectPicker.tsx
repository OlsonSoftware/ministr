import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown, Plus } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { popIn } from "../../lib/motion";
import { cn } from "../../lib/utils";
import { corpusLabel, corpusRoot } from "../../lib/corpus";
import { useCorpusFleet } from "../../lib/corpusFleet";
import { corpusTone, isCorpusLive } from "../../lib/status";
import { StatusDot } from "../ui/status-dot";
import type { CorpusInfo } from "../../lib/types";

interface Props {
  corpora: CorpusInfo[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onAddProject: () => void;
}

/** Persistent project switcher in the top bar — Cockpit styling. */
export function ProjectPicker({
  corpora,
  activeId,
  onSelect,
  onAddProject,
}: Props) {
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

  // Single source of truth: the shared fleet view model carries live,
  // phase-aware indexing state so the always-visible pill reflects it.
  const { byId } = useCorpusFleet(corpora);
  const active = corpora.find((c) => c.id === activeId) ?? null;
  const activeVm = activeId ? byId[activeId] : undefined;

  const triggerCls = cn(
    "inline-flex items-center gap-2 px-3 h-8 cursor-pointer rounded-md max-w-[280px]",
    "border border-border bg-surface text-text",
    "hover:bg-surface-overlay hover:border-border-hover",
    "transition-colors duration-150",
  );

  if (corpora.length === 0) {
    return (
      <button type="button" onClick={onAddProject} className={triggerCls}>
        <Plus className="h-3.5 w-3.5" strokeWidth={2} />
        <span className="font-sans text-xs font-medium">Add project</span>
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
        className={triggerCls}
      >
        {active && (
          <StatusDot
            tone={corpusTone(active)}
            pulse={isCorpusLive(active) ? "live" : "off"}
          />
        )}
        <span className="font-mono text-xs font-medium truncate">
          {active ? corpusLabel(active) : "Select project"}
        </span>
        {activeVm?.isIndexing && (
          <span className="font-mono text-mono-mini tabular-nums text-text-dim shrink-0">
            {Math.round(activeVm.primary.pct)}%
          </span>
        )}
        <ChevronDown
          className={cn(
            "h-3.5 w-3.5 shrink-0 transition-transform duration-150",
            open && "rotate-180",
          )}
          strokeWidth={2}
        />
      </button>

      <AnimatePresence>
        {open && (
          <motion.div
            role="listbox"
            variants={popIn}
            initial="initial"
            animate="animate"
            exit="exit"
            className={cn(
              "absolute top-full left-0 mt-2 z-50 origin-top-left",
              "min-w-[280px] max-w-[420px] overflow-hidden",
              "rounded-lg border border-border bg-surface shadow-lg",
            )}
          >
            <ul className="max-h-[320px] overflow-y-auto p-1">
              {corpora.map((c) => {
                const isActive = c.id === activeId;
                const root = corpusRoot(c.paths);
                const vm = byId[c.id];
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
                        "w-full flex items-center gap-2 px-2.5 py-2 text-left rounded-md cursor-pointer",
                        "transition-colors duration-150",
                        isActive
                          ? "bg-surface-overlay text-text"
                          : "text-text-muted hover:bg-surface-overlay hover:text-text",
                      )}
                    >
                      <Check
                        className={cn(
                          "h-3.5 w-3.5 shrink-0",
                          isActive ? "text-accent" : "text-transparent",
                        )}
                        strokeWidth={3}
                      />
                      <StatusDot
                        tone={corpusTone(c)}
                        pulse={isCorpusLive(c) ? "live" : "off"}
                      />
                      <div className="flex-1 min-w-0">
                        <div className="font-mono text-xs font-medium truncate">
                          {corpusLabel(c)}
                        </div>
                        {root && (
                          <div className="font-mono text-mono-mini text-text-dim truncate">
                            {root}
                          </div>
                        )}
                      </div>
                      {vm?.isIndexing && (
                        <span className="font-mono text-mono-mini tabular-nums text-text-dim shrink-0">
                          {Math.round(vm.primary.pct)}%
                        </span>
                      )}
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
                "w-full flex items-center gap-2 px-3 py-2.5 cursor-pointer",
                "border-t border-border bg-surface-overlay text-text",
                "hover:bg-surface font-sans text-xs font-medium",
                "transition-colors duration-150",
              )}
            >
              <Plus className="h-3.5 w-3.5" strokeWidth={2} />
              Add project
            </button>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
