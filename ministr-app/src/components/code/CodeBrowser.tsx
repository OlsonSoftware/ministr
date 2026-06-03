/**
 * CodeBrowser — the Code surface compound.
 *
 * Composition root for symbol-navigable code reading: a file tree, a
 * Shiki-highlighted viewer with clickable symbols, an inline peek /
 * references panel, and a ⌘K symbol-jump palette. It owns orchestration only
 * — each concern lives in its own module (SRP); this file wires them to the
 * same symbol graph the AI uses (read_file / search_symbols /
 * symbol_definition / symbol_references).
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AnimatePresence, motion } from "motion/react";
import {
  ArrowLeft,
  Cable,
  Code2,
  Command,
  PanelRight,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react";
import type {
  DaemonStatus,
  FileContent,
  Occurrence,
  SymbolInfo,
  SymbolRef,
} from "../../lib/types";
import { spring } from "../../lib/motion";
import { cn } from "../../lib/utils";
import { FileTree } from "./FileTree";
import { CodeViewer } from "./CodeViewer";
import { CodeLanding } from "./CodeLanding";
import { BridgeMapConnector } from "./BridgeMap";
import { DeadCodeMapConnector } from "./DeadCodeMap";
import { SolidMapConnector } from "./SolidMap";
import { SymbolNeighborhoodConnector } from "./SymbolNeighborhood";
import { RelatedFilesPanel } from "./RelatedFilesPanel";
import { SymbolPalette } from "./SymbolPalette";
import { useCodeNavigation } from "./useCodeNavigation";
import { useColorScheme } from "./useColorScheme";
import { useContainerWidth } from "./useContainerWidth";

/**
 * Surface width (px) at/above which the right panel is an inline third column;
 * below it, the panel becomes a slide-over drawer so the code viewer keeps its
 * width on narrow windows. Measured on the surface itself (rail already
 * excluded), not the window.
 */
const WIDE_PANEL_PX = 1040;

interface Props {
  status: DaemonStatus;
  activeCorpusId: string | null;
}

interface PanelState {
  mode: "peek" | "refs";
  symbolId: string;
  symbolName: string;
}

export function CodeBrowser({ status, activeCorpusId }: Props) {
  const corpusId = activeCorpusId ?? status.corpora[0]?.id ?? "";
  const corpus = status.corpora.find((c) => c.id === corpusId) ?? null;
  const scheme = useColorScheme();
  const nav = useCodeNavigation();
  const [file, setFile] = useState<FileContent | null>(null);
  const [occurrences, setOccurrences] = useState<Occurrence[]>([]);
  const [fileLoading, setFileLoading] = useState(false);
  const [panel, setPanel] = useState<PanelState | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  // Narrow-width drawer: when the right panel can't be an inline column, it
  // slides over instead. `rightOpen` gates that drawer (no effect when wide).
  const [rightOpen, setRightOpen] = useState(false);
  // The Explore lens: the code browser, the cross-language bridge map, or the
  // unused-symbol (dead-code) map.
  const [lens, setLens] = useState<Lens>("code");

  const gridRef = useRef<HTMLDivElement>(null);
  const surfaceWidth = useContainerWidth(gridRef);
  const isWide = surfaceWidth >= WIDE_PANEL_PX;

  const path = nav.current?.path ?? null;

  // Reset everything when the active corpus changes.
  useEffect(() => {
    nav.reset();
    setFile(null);
    setPanel(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [corpusId]);

  // Load the current file.
  useEffect(() => {
    if (!corpusId || !path) {
      setFile(null);
      return;
    }
    let cancelled = false;
    setFileLoading(true);
    invoke<FileContent>("read_file", { corpusId, path })
      .then((f) => {
        if (!cancelled) setFile(f);
      })
      .catch(() => {
        if (!cancelled) setFile(null);
      })
      .finally(() => {
        if (!cancelled) setFileLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [corpusId, path]);

  // Load the v2 occurrence index for the current file (empty unless the corpus
  // opted into occurrence indexing — the viewer then falls back to def spans).
  useEffect(() => {
    if (!corpusId || !path) {
      setOccurrences([]);
      return;
    }
    let cancelled = false;
    invoke<Occurrence[]>("file_occurrences", { corpusId, path })
      .then((o) => {
        if (!cancelled) setOccurrences(o);
      })
      .catch(() => {
        if (!cancelled) setOccurrences([]);
      });
    return () => {
      cancelled = true;
    };
  }, [corpusId, path]);

  // ⌘K opens the symbol palette while the Code surface is mounted. Capture
  // phase + stopPropagation so it overrides the global command palette here.
  useEffect(() => {
    function onKeyCapture(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        e.stopPropagation();
        setPaletteOpen(true);
      }
    }
    window.addEventListener("keydown", onKeyCapture, true);
    return () => window.removeEventListener("keydown", onKeyCapture, true);
  }, []);

  const focusLine = useMemo(() => {
    const loc = nav.current;
    if (!loc || !file || file.path !== loc.path) return undefined;
    if (loc.line) return loc.line;
    if (loc.symbolId) {
      return file.symbol_spans.find((s) => s.id === loc.symbolId)?.line_start;
    }
    return undefined;
  }, [nav, file]);

  const openSymbol = useCallback((symbolId: string, name: string) => {
    setPanel({ mode: "peek", symbolId, symbolName: name });
    setRightOpen(true); // surface the peek immediately, drawer or column
  }, []);

  // Close the right panel: drop any symbol peek and dismiss the narrow drawer.
  const closeRight = useCallback(() => {
    setPanel(null);
    setRightOpen(false);
  }, []);

  // Escape closes the narrow drawer (when the symbol palette isn't up).
  useEffect(() => {
    if (isWide || !rightOpen) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape" && !paletteOpen) {
        e.stopPropagation();
        setRightOpen(false);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isWide, rightOpen, paletteOpen]);

  const goToDefinition = useCallback(
    (filePath: string, line: number) => {
      nav.push({ path: filePath, line });
    },
    [nav],
  );

  // A reference edge carries names + files, not lines — resolve the caller's
  // line via search_symbols (the precise+search model), then navigate.
  const jumpToRef = useCallback(
    async (ref: SymbolRef) => {
      try {
        const matches = await invoke<SymbolInfo[]>("search_symbols", {
          corpusId,
          query: ref.from_name,
          kind: null,
          filePath: ref.from_file,
        });
        const exact = matches.find((m) => m.name === ref.from_name) ?? matches[0];
        nav.push(
          exact
            ? { path: ref.from_file, symbolId: exact.id }
            : { path: ref.from_file },
        );
      } catch {
        nav.push({ path: ref.from_file });
      }
    },
    [corpusId, nav],
  );

  const pickFromPalette = useCallback(
    (sym: SymbolInfo) => {
      setPaletteOpen(false);
      nav.push({ path: sym.file_path, symbolId: sym.id });
      setPanel({ mode: "peek", symbolId: sym.id, symbolName: sym.name });
    },
    [nav],
  );

  if (!corpusId) {
    return (
      <div className="grid h-full place-items-center">
        <div className="flex flex-col items-center gap-2 text-center">
          <Code2 className="h-8 w-8 text-text-dim" strokeWidth={1.5} />
          <p className="font-sans text-sm text-text">No project selected</p>
          <p className="max-w-xs font-sans text-xs text-text-dim">
            Pick a project from the top bar to browse its code.
          </p>
        </div>
      </div>
    );
  }

  // The right-panel content, built once and placed in exactly one location
  // (inline column when wide, slide-over drawer when narrow): a clicked
  // symbol's peek/references, else the open file's related files.
  const rightPanel = panel ? (
    <SymbolNeighborhoodConnector
      corpusId={corpusId}
      symbolId={panel.symbolId}
      symbolName={panel.symbolName}
      onGoToDefinition={goToDefinition}
      onJumpRef={jumpToRef}
      onClose={closeRight}
    />
  ) : file ? (
    <RelatedFilesPanel
      corpusId={corpusId}
      file={file}
      onOpen={(p) => nav.push({ path: p })}
    />
  ) : null;

  return (
    <div className="@container/page relative flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 items-center gap-2 border-b border-border-soft bg-surface px-3 py-1.5">
        <LensToggle lens={lens} onChange={setLens} />
        <div className="h-4 w-px bg-border-soft" aria-hidden />
        {lens === "code" ? (
          <>
            {nav.canBack && (
              <button
                type="button"
                onClick={nav.back}
                aria-label="Back"
                className="grid h-6 w-6 place-items-center rounded-md border border-border-soft text-text-muted hover:border-border hover:text-text cursor-pointer transition-colors duration-150 ease-out"
              >
                <ArrowLeft className="h-3 w-3" strokeWidth={2} />
              </button>
            )}
            <span className="truncate font-mono text-xs text-text-muted">
              {path ?? "Select a file"}
            </span>
          </>
        ) : (
          <span className="truncate font-mono text-xs text-text-muted">
            {lens === "bridges"
              ? "Cross-language seams"
              : lens === "unused"
                ? "Unused candidates"
                : "Architecture findings"}
          </span>
        )}
        <div className="ml-auto flex items-center gap-2">
          {lens === "code" && !isWide && file && (
            <button
              type="button"
              onClick={() => setRightOpen((o) => !o)}
              aria-label={rightOpen ? "Hide side panel" : "Show side panel"}
              aria-pressed={rightOpen}
              title="Symbol & related-file panel"
              className={cn(
                "inline-flex items-center gap-1 rounded-md border px-2 py-1 font-mono text-mono-mini cursor-pointer transition-colors duration-150 ease-out",
                rightOpen
                  ? "border-accent bg-surface-overlay text-text"
                  : "border-border-soft text-text-muted hover:border-border hover:text-text",
              )}
            >
              <PanelRight className="h-3 w-3" strokeWidth={2} />
            </button>
          )}
          {lens === "code" && (
            <button
              type="button"
              onClick={() => setPaletteOpen(true)}
              className="inline-flex items-center gap-1 rounded-md border border-border-soft px-2 py-1 font-mono text-mono-mini text-text-muted hover:border-border hover:text-text cursor-pointer transition-colors duration-150 ease-out"
            >
              <Command className="h-3 w-3" strokeWidth={2} />
              <span>K</span>
              <span className="text-text-dim">jump to symbol</span>
            </button>
          )}
        </div>
      </header>

      {lens === "bridges" ? (
        <div className="min-h-0 flex-1">
          <BridgeMapConnector
            corpusId={corpusId}
            onOpenFile={(p) => {
              setLens("code");
              nav.push({ path: p });
            }}
          />
        </div>
      ) : lens === "unused" ? (
        <div className="min-h-0 flex-1">
          <DeadCodeMapConnector
            corpusId={corpusId}
            onOpenFile={(p, line) => {
              setLens("code");
              nav.push({ path: p, line });
            }}
          />
        </div>
      ) : lens === "solid" ? (
        <div className="min-h-0 flex-1">
          <SolidMapConnector corpusId={corpusId} />
        </div>
      ) : (
      <div
        ref={gridRef}
        className={cn(
          "grid min-h-0 flex-1",
          isWide
            ? "grid-cols-[clamp(200px,20%,300px)_minmax(0,1fr)_clamp(300px,28%,420px)]"
            : "grid-cols-[clamp(200px,26%,300px)_minmax(0,1fr)]",
        )}
      >
        <FileTree corpusId={corpusId} activePath={path} onSelect={(p) => nav.push({ path: p })} />

        <div className="min-h-0 min-w-0 border-r border-border-soft">
          {!path ? (
            <CodeLanding
              corpusId={corpusId}
              corpus={corpus}
              onOpen={(p) => nav.push({ path: p })}
            />
          ) : fileLoading && !file ? (
            <div className="grid h-full place-items-center">
              <span className="font-mono text-sm text-text-dim">Loading_</span>
            </div>
          ) : file ? (
            <CodeViewer
              file={file}
              scheme={scheme}
              focusLine={focusLine}
              occurrences={occurrences}
              onSymbolClick={openSymbol}
            />
          ) : (
            <div className="grid h-full place-items-center px-6 text-center">
              <p className="font-mono text-sm text-text-dim">Couldn’t read {path}.</p>
            </div>
          )}
        </div>

        {/* Wide: the right panel is an inline third column. The content
            (symbol peek/references, or the file's related files) mounts in
            exactly one place — here or the narrow drawer below — so its data
            fetches never double-fire. */}
        {isWide && rightPanel && (
          <aside className="min-h-0 min-w-0 bg-surface">{rightPanel}</aside>
        )}
      </div>
      )}

      {/* Narrow: the same panel slides over the viewer instead of stealing its
          width. Backdrop + Escape (above) + the X dismiss it. */}
      <AnimatePresence>
        {!isWide && rightOpen && rightPanel && (
          <motion.div
            className="absolute inset-0 z-30 flex"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
          >
            <button
              type="button"
              aria-label="Close panel"
              onClick={closeRight}
              className="flex-1 bg-bg/60 cursor-default"
            />
            <motion.aside
              className="flex min-h-0 w-[min(420px,85%)] flex-col border-l border-border bg-surface shadow-[var(--glow-soft)]"
              initial={{ x: 24 }}
              animate={{ x: 0 }}
              exit={{ x: 24 }}
              transition={spring}
            >
              <div className="flex shrink-0 items-center justify-end border-b border-border-soft px-2 py-1">
                <button
                  type="button"
                  onClick={closeRight}
                  aria-label="Close panel"
                  className="grid h-5 w-5 place-items-center rounded-md border border-border-soft text-text-muted hover:border-border hover:text-text cursor-pointer transition-colors duration-150 ease-out"
                >
                  <X className="h-2.5 w-2.5" strokeWidth={2} />
                </button>
              </div>
              <div className="min-h-0 flex-1">{rightPanel}</div>
            </motion.aside>
          </motion.div>
        )}
      </AnimatePresence>

      <SymbolPalette
        open={paletteOpen}
        corpusId={corpusId}
        onClose={() => setPaletteOpen(false)}
        onPick={pickFromPalette}
      />
    </div>
  );
}

type Lens = "code" | "bridges" | "unused" | "solid";

/** Code | Bridges | Unused lens switch — the three ways to read the index:
 *  file-by-file, by its cross-language seams, or by what nothing references
 *  (dead-code candidates). A segmented control in the Explore header. */
function LensToggle({
  lens,
  onChange,
}: {
  lens: Lens;
  onChange: (l: Lens) => void;
}) {
  const items: Array<{ id: Lens; label: string; icon: typeof Code2 }> = [
    { id: "code", label: "Code", icon: Code2 },
    { id: "bridges", label: "Bridges", icon: Cable },
    { id: "unused", label: "Unused", icon: Trash2 },
    { id: "solid", label: "Quality", icon: ShieldCheck },
  ];
  return (
    <div
      role="tablist"
      aria-label="Explore lens"
      className="inline-flex items-center gap-0.5 rounded-md border border-border-soft bg-surface-sunken p-0.5"
    >
      {items.map(({ id, label, icon: Icon }) => {
        const active = lens === id;
        return (
          <button
            key={id}
            type="button"
            role="tab"
            aria-selected={active}
            onClick={() => onChange(id)}
            className={cn(
              "inline-flex items-center gap-1 rounded px-2 py-0.5 font-mono text-mono-mini font-semibold uppercase tracking-[0.06em] cursor-pointer transition-colors duration-150 ease-out",
              active
                ? "bg-surface-overlay text-text shadow-[var(--glow-soft)]"
                : "text-text-dim hover:text-text",
            )}
          >
            <Icon className="h-3 w-3" strokeWidth={2.25} />
            {label}
          </button>
        );
      })}
    </div>
  );
}
