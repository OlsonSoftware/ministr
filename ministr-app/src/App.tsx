import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Search, AlertTriangle } from "lucide-react";
import {
  BrutalSearch,
  BrutalAsk,
  BrutalSymbols,
  BrutalBridge,
  BrutalProjects,
  BrutalStructure,
  BrutalSessions,
  BrutalLogs,
  BrutalSettings,
} from "./components/ui/brutal-icons";
import { useDaemonStatus } from "./hooks/useDaemonStatus";
import { useTheme } from "./hooks/useTheme";
import { useCorpusContext } from "./hooks/useCorpusContext";
import { useDefaultTab, useDensity } from "./hooks/usePreferences";
import { ProjectList } from "./components/ProjectList";
import { ProjectDetail } from "./components/ProjectDetail";
import { Settings } from "./components/Settings";
import { LogViewer } from "./components/LogViewer";
import { Onboarding } from "./components/Onboarding";
import { SessionDashboard } from "./components/SessionDashboard";
import { QueryPlayground } from "./components/QueryPlayground";
import { CorpusTreemap } from "./components/CorpusTreemap";
import { SymbolGraph } from "./components/SymbolGraph";
import { Bridge } from "./components/Bridge";
import { ContextSimulator } from "./components/ContextSimulator";
import { AskView } from "./components/AskView";
import { CommandPalette } from "./components/CommandPalette";
import { ShortcutSheet } from "./components/ShortcutSheet";
import { CorpusPill } from "./components/shell/CorpusPill";
import { DaemonDot } from "./components/shell/DaemonDot";
import { VitalsChip } from "./components/shell/VitalsChip";
import { ToastProvider, useToast } from "./components/shell/ToastTray";
import { EntityPanelProvider } from "./hooks/useEntityPanel";
import { EntityPanel } from "./components/EntityPanel";
import { cn } from "./lib/utils";
import {
  matchShortcut,
  firesWhileTyping,
  type ShortcutAction,
} from "./lib/shortcuts";

type Tab =
  | "search"
  | "ask"
  | "symbols"
  | "bridge"
  | "projects"
  | "structure"
  | "sessions"
  | "simulator"
  | "logs"
  | "settings";

const VALID_TABS: Tab[] = [
  "search",
  "ask",
  "symbols",
  "bridge",
  "projects",
  "structure",
  "sessions",
  "simulator",
  "logs",
  "settings",
];

export function App() {
  return (
    <ToastProvider>
      <EntityPanelProvider>
        <AppInner />
      </EntityPanelProvider>
    </ToastProvider>
  );
}

function AppInner() {
  const { status, error, refresh } = useDaemonStatus();
  const { theme, setTheme } = useTheme();
  const { activeCorpus, activeCorpusId, setActiveCorpusId } =
    useCorpusContext(status);
  const { defaultTab } = useDefaultTab();
  // Initialize density preference (sets data-density on <html>).
  useDensity();
  const { toast } = useToast();
  const [tab, setTab] = useState<Tab>(defaultTab as Tab);
  const [showOnboarding, setShowOnboarding] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [railCollapsed, setRailCollapsed] = useState(false);
  const gPending = useRef(false);
  const gTimer = useRef<number | null>(null);

  useEffect(() => {
    invoke<boolean>("should_show_onboarding").then(setShowOnboarding);
  }, []);

  useEffect(() => {
    const unlistenNav = listen<string>("navigate", (event) => {
      const target = event.payload as Tab;
      if (VALID_TABS.includes(target)) setTab(target);
    });
    const unlistenSelect = listen<string>("select-corpus", (event) => {
      if (typeof event.payload === "string") {
        setActiveCorpusId(event.payload);
      }
    });
    // In-app navigation requests from components (e.g. LogViewer deep-links).
    function onWindowNavigate(e: Event) {
      const detail = (e as CustomEvent).detail;
      if (typeof detail === "string" && VALID_TABS.includes(detail as Tab)) {
        setTab(detail as Tab);
      }
    }
    window.addEventListener("ministr-navigate", onWindowNavigate);
    return () => {
      unlistenNav.then((fn) => fn());
      unlistenSelect.then((fn) => fn());
      window.removeEventListener("ministr-navigate", onWindowNavigate);
    };
  }, [setActiveCorpusId]);

  // Global keyboard shortcuts
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null;
      const typing =
        target?.tagName === "INPUT" ||
        target?.tagName === "TEXTAREA" ||
        target?.isContentEditable;

      // Single source of truth: the shortcut config decides what happens.
      const result = matchShortcut(e, gPending.current);

      // First pass — actions flagged firesWhileTyping bypass the typing
      // bail below (currently just ⌘K / Ctrl+K so the palette is always
      // reachable from inside any input).
      if (
        result &&
        result !== "_pending:g" &&
        firesWhileTyping(result)
      ) {
        e.preventDefault();
        dispatchShortcut(result);
        return;
      }

      if (typing) return;

      if (e.key === "Escape") {
        // Stop propagation so EntityPanel's window-level Esc handler
        // (mounted from useEntityPanel) doesn't ALSO fire and close the
        // entity drawer underneath whichever overlay we just dismissed.
        // Topmost-modal-first wins.
        if (paletteOpen) {
          e.preventDefault();
          e.stopImmediatePropagation();
          setPaletteOpen(false);
        } else if (shortcutsOpen) {
          e.preventDefault();
          e.stopImmediatePropagation();
          setShortcutsOpen(false);
        }
        return;
      }

      if (result === "_pending:g") {
        gPending.current = true;
        if (gTimer.current !== null) clearTimeout(gTimer.current);
        gTimer.current = window.setTimeout(() => {
          gPending.current = false;
        }, 900);
        return;
      }
      if (gPending.current) {
        gPending.current = false;
        if (gTimer.current !== null) clearTimeout(gTimer.current);
      }
      if (!result) return;
      e.preventDefault();
      dispatchShortcut(result);

      function dispatchShortcut(action: ShortcutAction) {
        switch (action) {
          case "toggle:shortcuts":
            setShortcutsOpen((o) => !o);
            return;
          case "toggle:rail":
            setRailCollapsed((c) => !c);
            return;
          case "nav:search":
            setTab("search");
            return;
          case "nav:ask":
            setTab("ask");
            return;
          case "nav:symbols":
            setTab("symbols");
            return;
          case "nav:bridge":
            setTab("bridge");
            return;
          case "nav:projects":
            setTab("projects");
            return;
          case "nav:structure":
            setTab("structure");
            return;
          case "nav:sessions":
            setTab("sessions");
            return;
          case "nav:logs":
            setTab("logs");
            return;
          case "nav:settings":
            setTab("settings");
            return;
          // toggle:palette handled in the meta+K branch above.
          case "toggle:palette":
            setPaletteOpen((o) => !o);
            return;
        }
      }
    }

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [paletteOpen, shortcutsOpen]);

  const openAddProject = useCallback(async () => {
    try {
      await invoke("add_project_dialog");
      refresh();
      toast("PROJECT ADDED", { tone: "success" });
    } catch {
      /* ignore */
    }
  }, [refresh, toast]);

  function onSelectCorpus(id: string) {
    const c = status?.corpora.find((x) => x.id === id);
    setActiveCorpusId(id);
    if (c) toast("CORPUS", { detail: c.id, tone: "info" });
  }

  function onThemeChange(t: "system" | "dark" | "light") {
    setTheme(t);
    toast("THEME", { detail: t.toUpperCase(), tone: "info" });
  }

  if (showOnboarding) {
    return <Onboarding onDismiss={() => setShowOnboarding(false)} />;
  }

  return (
    <div className="flex h-screen flex-col bg-bg text-text">
      <TopBar
        status={status}
        error={error}
        activeCorpus={activeCorpus}
        onSelectCorpus={onSelectCorpus}
        onPaletteOpen={() => setPaletteOpen(true)}
        onShortcutsOpen={() => setShortcutsOpen(true)}
        onOpenLogs={async () => {
          // The DaemonDot popover button reads "Open log file" — that
          // promise is the *file on disk*, not the Logs tab. Hand the
          // log path off to the OS opener and surface the toast based
          // on the actual outcome, so a failed open doesn't announce
          // success. Always switch to the in-app Logs view too — the
          // user wants to see the log either way.
          if (status?.log_path) {
            try {
              await invoke("open_path", { path: status.log_path });
              toast("Open log file", {
                detail: status.log_path,
                tone: "info",
              });
            } catch (e) {
              console.error("open_path(log) failed", e);
              toast("Could not open log file", {
                detail: status.log_path,
                tone: "danger",
              });
            }
          }
          setTab("logs");
        }}
      />

      {error && (
        <div className="flex items-center gap-2 border-b-2 border-danger bg-surface px-5 py-2 text-xs font-mono tracking-[0.05em] text-danger shrink-0">
          <AlertTriangle className="h-3.5 w-3.5 shrink-0" strokeWidth={2.5} />
          <span>{error}</span>
        </div>
      )}

      <div className="flex flex-1 min-h-0">
        <Rail tab={tab} onSelect={setTab} collapsed={railCollapsed} />

        <main className="flex-1 overflow-y-auto p-5">
          {!status ? (
            <ConnectingState error={error ?? null} />
          ) : tab === "search" ? (
            <QueryPlayground
              status={status}
              activeCorpusId={activeCorpusId}
              setActiveCorpusId={setActiveCorpusId}
            />
          ) : tab === "ask" ? (
            <AskView
              status={status}
              activeCorpusId={activeCorpusId}
              setActiveCorpusId={setActiveCorpusId}
            />
          ) : tab === "symbols" ? (
            <SymbolGraph
              status={status}
              activeCorpusId={activeCorpusId}
              setActiveCorpusId={setActiveCorpusId}
            />
          ) : tab === "bridge" ? (
            <Bridge
              status={status}
              activeCorpusId={activeCorpusId}
              setActiveCorpusId={setActiveCorpusId}
            />
          ) : tab === "projects" ? (
            <div className="@container/page flex gap-4 h-full min-h-0">
              <div
                className={cn(
                  "flex-1 min-w-0 min-h-0 overflow-y-auto",
                  activeCorpus &&
                    "@min-[1024px]/page:max-w-[clamp(360px,55%,720px)]",
                )}
              >
                <ProjectList
                  corpora={status.corpora}
                  onRefresh={refresh}
                  onSelect={setActiveCorpusId}
                  selectedId={activeCorpusId}
                />
              </div>
              {activeCorpus && (
                <div className="flex-1 min-w-0 min-h-0 overflow-y-auto hidden @min-[1024px]/page:block">
                  <ProjectDetail
                    corpus={activeCorpus}
                    status={status}
                    onNavigate={(target) => setTab(target)}
                  />
                </div>
              )}
            </div>
          ) : tab === "structure" ? (
            <CorpusTreemap
              status={status}
              activeCorpusId={activeCorpusId}
              setActiveCorpusId={setActiveCorpusId}
              onNavigate={(target) => setTab(target)}
            />
          ) : tab === "sessions" ? (
            <SessionDashboard status={status} />
          ) : tab === "simulator" ? (
            <ContextSimulator />
          ) : tab === "logs" ? (
            <LogViewer />
          ) : (
            <Settings
              status={status}
              theme={theme}
              onThemeChange={onThemeChange}
              onShowOnboarding={() => setShowOnboarding(true)}
              onRefresh={refresh}
              onOpenLogs={() => setTab("logs")}
            />
          )}
        </main>
      </div>

      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        status={status}
        onNavigate={(t) => setTab(t as Tab)}
        onAddProject={openAddProject}
        onSelectCorpus={onSelectCorpus}
        onShowShortcuts={() => setShortcutsOpen(true)}
        onThemeChange={onThemeChange}
        onRefresh={refresh}
      />
      <ShortcutSheet
        open={shortcutsOpen}
        onClose={() => setShortcutsOpen(false)}
      />

      {/* Universal entity-detail drawer — provider lives above us, panel
          renders here so it overlays every page. */}
      <EntityPanel />
    </div>
  );
}

function TopBar({
  status,
  error,
  activeCorpus,
  onSelectCorpus,
  onPaletteOpen,
  onShortcutsOpen,
  onOpenLogs,
}: {
  status: import("./lib/types").DaemonStatus | null;
  error: string | null;
  activeCorpus: import("./lib/types").CorpusInfo | null;
  onSelectCorpus: (id: string) => void;
  onPaletteOpen: () => void;
  onShortcutsOpen: () => void;
  onOpenLogs: () => void;
}) {
  const totalSymbols = status?.corpora.reduce(
    (s, c) => s + (c.symbols_count ?? 0),
    0,
  );
  return (
    <header className="flex items-center justify-between gap-4 border-b-2 border-border bg-surface px-5 py-2.5 shrink-0">
      <div className="flex items-center gap-3 min-w-0">
        <span
          className="ministr-wordmark"
          title={status ? `ministr v${status.version}` : "ministr"}
        >
          ministr
        </span>
        <span className="font-mono text-xs font-semibold tracking-[0.05em] text-text-dim hidden md:inline">
          CODE INTELLIGENCE
        </span>
        <DaemonDot status={status} error={error} onOpenLogs={onOpenLogs} />
        <span className="hidden md:inline-block w-px h-4 bg-border opacity-50" />
        <CorpusPill
          corpora={status?.corpora ?? []}
          activeCorpus={activeCorpus}
          onSelect={onSelectCorpus}
        />
      </div>

      {status && (
        <div className="flex items-center gap-1 min-w-0">
          {/* Sessions chip is highest-signal — always visible. */}
          {status.total_sessions > 0 && (
            <VitalsChip
              label="SESSIONS"
              value={status.total_sessions}
              accent
            />
          )}
          {/* Below xl: drop CORPORA + SYMBOLS to save horizontal space. */}
          <span className="hidden xl:inline-flex">
            <VitalsChip label="CORPORA" value={status.corpora.length} />
          </span>
          {totalSymbols !== undefined && totalSymbols > 0 && (
            <span className="hidden xl:inline-flex">
              <VitalsChip
                label="SYMBOLS"
                value={totalSymbols.toLocaleString()}
              />
            </span>
          )}
          {/* Below lg: drop MEM. */}
          <span className="hidden lg:inline-flex">
            <VitalsChip label="MEM" value={`${status.memory_mb.toFixed(0)}MB`} />
          </span>
        </div>
      )}

      <div className="flex items-center gap-2 shrink-0">
        <button
          onClick={onPaletteOpen}
          title="Command palette (⌘K)"
          className="inline-flex items-center gap-2 border border-border-soft bg-surface px-2.5 py-1 text-sm font-sans font-medium text-text-muted hover:text-text hover:border-border cursor-pointer transition-none"
          style={{ borderRadius: "var(--radius-button)" }}
        >
          <Search className="h-3.5 w-3.5" strokeWidth={2} />
          {/* Hide the "Search" text below md so the button collapses to icon + ⌘K. */}
          <span className="hidden md:inline">Search</span>
          <kbd
            className="border border-border-soft bg-surface-overlay px-1 text-[0.6875rem] font-mono text-text-dim"
            style={{ borderRadius: "var(--radius-pill)" }}
          >
            ⌘K
          </kbd>
        </button>
        <button
          onClick={onShortcutsOpen}
          title="Shortcuts (?)"
          className="inline-flex h-7 items-center justify-center border border-border-soft bg-surface px-2 text-sm font-serif font-normal text-text-muted hover:text-text hover:border-border cursor-pointer transition-none"
          style={{ borderRadius: "var(--radius-button)" }}
        >
          ?
        </button>
      </div>
    </header>
  );
}

function Rail({
  tab,
  onSelect,
  collapsed,
}: {
  tab: Tab;
  onSelect: (t: Tab) => void;
  collapsed: boolean;
}) {
  if (collapsed) return null;

  return (
    <nav className="hidden sm:flex flex-col w-14 border-r border-border bg-surface py-3 items-center gap-1 shrink-0">
      <RailItem
        icon={BrutalSearch}
        active={tab === "search"}
        label="Search"
        onClick={() => onSelect("search")}
      />
      <RailItem
        icon={BrutalAsk}
        active={tab === "ask"}
        label="Ask"
        onClick={() => onSelect("ask")}
      />
      <RailItem
        icon={BrutalSymbols}
        active={tab === "symbols"}
        label="Symbols"
        onClick={() => onSelect("symbols")}
      />
      <RailItem
        icon={BrutalBridge}
        active={tab === "bridge"}
        label="Bridge"
        onClick={() => onSelect("bridge")}
      />
      <RailItem
        icon={BrutalProjects}
        active={tab === "projects"}
        label="Projects"
        onClick={() => onSelect("projects")}
      />
      <RailItem
        icon={BrutalStructure}
        active={tab === "structure"}
        label="Structure"
        onClick={() => onSelect("structure")}
      />
      <RailItem
        icon={BrutalSessions}
        active={tab === "sessions"}
        label="Sessions"
        onClick={() => onSelect("sessions")}
      />
      <RailItem
        icon={BrutalLogs}
        active={tab === "logs"}
        label="Logs"
        onClick={() => onSelect("logs")}
      />
      <div className="flex-1" />
      <RailItem
        icon={BrutalSettings}
        active={tab === "settings"}
        label="Settings"
        onClick={() => onSelect("settings")}
      />
    </nav>
  );
}

function RailItem({
  icon: Icon,
  active,
  onClick,
  label,
}: {
  icon: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  active: boolean;
  onClick: () => void;
  label: string;
}) {
  return (
    <button
      onClick={onClick}
      title={label}
      aria-label={label}
      className={cn(
        "relative grid place-items-center h-10 w-10 cursor-pointer transition-none",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
        active
          ? "bg-surface-overlay text-text"
          : "text-text-dim hover:text-text hover:bg-surface-overlay",
      )}
    >
      {active && (
        <span className="absolute -left-[2px] top-1/2 h-6 w-[3px] -translate-y-1/2 bg-accent" />
      )}
      <Icon className="h-[18px] w-[18px]" />
    </button>
  );
}

function ConnectingState({ error }: { error: string | null }) {
  return (
    <div className="flex flex-col items-center justify-center h-full gap-4">
      <div className="font-serif text-2xl font-normal text-text">
        Connecting<span className="ministr-blink">_</span>
      </div>
      {error && (
        <p className="max-w-md text-center text-sm font-sans text-danger">
          {error}
        </p>
      )}
    </div>
  );
}
