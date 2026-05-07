/**
 * ExploreView — unified retrieval surface that replaces the previous
 * Search, Symbols, and Bridge tabs.
 *
 * The three retrieval modes share the same daemon (sections / symbols /
 * bridges) and the same "type a query, look at results" workflow, so
 * collapsing them into one tab with a mode pivot is structurally
 * cleaner. Each mode still mounts its existing component as-is —
 * the only new code is a tiny pivot strip and the mode-switching
 * persistence.
 *
 * Mode is persisted globally (not per-corpus) under
 * `ministr-explore-mode` so power users keep their preferred default
 * while still being able to switch quickly.
 */

import { useEffect, useState } from "react";
import { Bridge } from "./Bridge";
import { QueryPlayground } from "./QueryPlayground";
import { SymbolGraph } from "./SymbolGraph";
import { cn } from "../lib/utils";
import type { DaemonStatus } from "../lib/types";

export type ExploreMode = "sections" | "symbols" | "bridges";

const MODE_STORAGE_KEY = "ministr-explore-mode";
const VALID_MODES: ExploreMode[] = ["sections", "symbols", "bridges"];

interface Props {
  status: DaemonStatus;
  activeCorpusId: string | null;
  setActiveCorpusId: (id: string | null) => void;
  /** Optional override — used when navigating in from another tab
   *  (e.g. CorpusTreemap onNavigate) to land on a specific mode. */
  initialMode?: ExploreMode;
}

export function ExploreView({
  status,
  activeCorpusId,
  setActiveCorpusId,
  initialMode,
}: Props) {
  const [mode, setMode] = useState<ExploreMode>(() => {
    if (initialMode && VALID_MODES.includes(initialMode)) return initialMode;
    try {
      const v = localStorage.getItem(MODE_STORAGE_KEY);
      if (v && VALID_MODES.includes(v as ExploreMode)) {
        return v as ExploreMode;
      }
    } catch {
      /* ignore */
    }
    return "sections";
  });

  // Honor an `initialMode` prop change (e.g., palette deep-link arrives
  // on a tab that's already mounted).
  useEffect(() => {
    if (initialMode && VALID_MODES.includes(initialMode)) {
      setMode(initialMode);
    }
  }, [initialMode]);

  // Persist mode whenever it changes — including initialMode-driven changes,
  // so the next bare `g e` lands the user where they last were.
  useEffect(() => {
    try {
      localStorage.setItem(MODE_STORAGE_KEY, mode);
    } catch {
      /* localStorage unavailable */
    }
  }, [mode]);

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      <ModePivot mode={mode} onChange={setMode} />
      <div className="flex-1 min-h-0">
        {mode === "sections" && (
          <QueryPlayground
            status={status}
            activeCorpusId={activeCorpusId}
            setActiveCorpusId={setActiveCorpusId}
          />
        )}
        {mode === "symbols" && (
          <SymbolGraph
            status={status}
            activeCorpusId={activeCorpusId}
            setActiveCorpusId={setActiveCorpusId}
          />
        )}
        {mode === "bridges" && (
          <Bridge
            status={status}
            activeCorpusId={activeCorpusId}
            setActiveCorpusId={setActiveCorpusId}
          />
        )}
      </div>
    </div>
  );
}

function ModePivot({
  mode,
  onChange,
}: {
  mode: ExploreMode;
  onChange: (m: ExploreMode) => void;
}) {
  const items: { id: ExploreMode; label: string; hint: string }[] = [
    { id: "sections", label: "Sections", hint: "Docs · code · prose" },
    { id: "symbols", label: "Symbols", hint: "Functions · structs · traits" },
    { id: "bridges", label: "Bridges", hint: "Tauri · FFI · HTTP" },
  ];
  return (
    <div className="flex items-stretch gap-0 border border-border-soft bg-surface shrink-0">
      {items.map((it, i) => {
        const active = mode === it.id;
        return (
          <button
            key={it.id}
            onClick={() => onChange(it.id)}
            className={cn(
              "flex-1 flex flex-col items-start gap-0.5 px-3 py-2 cursor-pointer transition-none text-left",
              i > 0 && "border-l border-border-soft",
              active
                ? "bg-surface-overlay text-text"
                : "text-text-dim hover:bg-surface-overlay hover:text-text",
            )}
          >
            <span
              className={cn(
                "font-mono text-mono-mini uppercase tracking-[0.05em]",
                active ? "text-accent" : "text-text-dim",
              )}
            >
              {it.label}
            </span>
            <span className="font-sans text-xs text-text-muted">
              {it.hint}
            </span>
          </button>
        );
      })}
    </div>
  );
}
