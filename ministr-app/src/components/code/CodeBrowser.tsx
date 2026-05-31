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
import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AnimatePresence, motion } from "motion/react";
import { ArrowLeft, Code2, Command } from "lucide-react";
import type { DaemonStatus, FileContent, SymbolInfo, SymbolRef } from "../../lib/types";
import { spring } from "../../lib/motion";
import { FileTree } from "./FileTree";
import { CodeViewer } from "./CodeViewer";
import { SymbolPeek } from "./SymbolPeek";
import { ReferencesPanel } from "./ReferencesPanel";
import { SymbolPalette } from "./SymbolPalette";
import { useCodeNavigation } from "./useCodeNavigation";
import { useColorScheme } from "./useColorScheme";

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
  const scheme = useColorScheme();
  const nav = useCodeNavigation();
  const [file, setFile] = useState<FileContent | null>(null);
  const [fileLoading, setFileLoading] = useState(false);
  const [panel, setPanel] = useState<PanelState | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);

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
  }, []);

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

  return (
    <div className="@container/page relative flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 items-center gap-2 border-b border-border-soft bg-surface px-3 py-1.5">
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
        <button
          type="button"
          onClick={() => setPaletteOpen(true)}
          className="ml-auto inline-flex items-center gap-1 rounded-md border border-border-soft px-2 py-1 font-mono text-mono-mini text-text-muted hover:border-border hover:text-text cursor-pointer transition-colors duration-150 ease-out"
        >
          <Command className="h-3 w-3" strokeWidth={2} />
          <span>K</span>
          <span className="text-text-dim">jump to symbol</span>
        </button>
      </header>

      <div className="grid min-h-0 flex-1 grid-cols-[clamp(200px,22%,300px)_minmax(0,1fr)] @min-[1100px]/page:grid-cols-[clamp(200px,20%,300px)_minmax(0,1fr)_clamp(300px,28%,420px)]">
        <FileTree corpusId={corpusId} activePath={path} onSelect={(p) => nav.push({ path: p })} />

        <div className="min-h-0 min-w-0 border-r border-border-soft">
          {!path ? (
            <CenterEmpty />
          ) : fileLoading && !file ? (
            <div className="grid h-full place-items-center">
              <span className="font-mono text-sm text-text-dim">Loading_</span>
            </div>
          ) : file ? (
            <CodeViewer
              file={file}
              scheme={scheme}
              focusLine={focusLine}
              onSymbolClick={openSymbol}
            />
          ) : (
            <div className="grid h-full place-items-center px-6 text-center">
              <p className="font-mono text-sm text-text-dim">Couldn’t read {path}.</p>
            </div>
          )}
        </div>

        <AnimatePresence>
          {panel && (
            <motion.aside
              key={`${panel.mode}-${panel.symbolId}`}
              initial={{ opacity: 0, x: 12 }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, x: 12 }}
              transition={spring}
              className="hidden min-h-0 min-w-0 bg-surface @min-[1100px]/page:block"
            >
              {panel.mode === "peek" ? (
                <SymbolPeek
                  corpusId={corpusId}
                  symbolId={panel.symbolId}
                  symbolName={panel.symbolName}
                  onGoToDefinition={goToDefinition}
                  onShowReferences={() =>
                    setPanel((p) => (p ? { ...p, mode: "refs" } : p))
                  }
                  onClose={() => setPanel(null)}
                />
              ) : (
                <ReferencesPanel
                  corpusId={corpusId}
                  symbolId={panel.symbolId}
                  symbolName={panel.symbolName}
                  onBack={() => setPanel((p) => (p ? { ...p, mode: "peek" } : p))}
                  onJump={jumpToRef}
                />
              )}
            </motion.aside>
          )}
        </AnimatePresence>
      </div>

      <SymbolPalette
        open={paletteOpen}
        corpusId={corpusId}
        onClose={() => setPaletteOpen(false)}
        onPick={pickFromPalette}
      />
    </div>
  );
}

function CenterEmpty() {
  return (
    <div className="grid h-full place-items-center px-6 text-center">
      <div className="flex flex-col items-center gap-2">
        <Code2 className="h-8 w-8 text-text-dim" strokeWidth={1.5} />
        <p className="font-sans text-sm text-text">Pick a file</p>
        <p className="max-w-xs font-sans text-xs text-text-dim">
          Choose a file from the tree, or press ⌘K to jump straight to a symbol.
          Click any underlined symbol to peek its definition and references.
        </p>
      </div>
    </div>
  );
}
