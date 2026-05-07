import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AlertTriangle } from "lucide-react";
import { useDaemonStatus } from "./hooks/useDaemonStatus";
import { useTheme } from "./hooks/useTheme";
import { useCorpusContext } from "./hooks/useCorpusContext";
import { useDensity } from "./hooks/usePreferences";
import { useInvestigations } from "./hooks/useInvestigations";
import { ProjectList } from "./components/ProjectList";
import { Settings } from "./components/Settings";
import { Onboarding } from "./components/Onboarding";
import { SessionDashboard } from "./components/SessionDashboard";
import { AskView } from "./components/AskView";
import { ExploreView, type ExploreMode } from "./components/ExploreView";
import { CommandPalette } from "./components/CommandPalette";
import { ShortcutSheet } from "./components/ShortcutSheet";
import { LogViewer } from "./components/LogViewer";
import { ToastProvider, useToast } from "./components/shell/ToastTray";
import { EntityPanelProvider } from "./hooks/useEntityPanel";
import { EntityPanel } from "./components/EntityPanel";
import { WorkspaceShell } from "./components/workspace/WorkspaceShell";
import { CorpusRail } from "./components/workspace/CorpusRail";
import { SourcePane } from "./components/workspace/SourcePane";
import { StatusBar } from "./components/workspace/StatusBar";
import { Drawer } from "./components/workspace/Drawer";
import { BrutalAsk, BrutalExplore } from "./components/ui/brutal-icons";
import { corpusLabel } from "./lib/corpus";
import { cn } from "./lib/utils";
import {
  matchShortcut,
  firesWhileTyping,
  type ShortcutAction,
} from "./lib/shortcuts";

/**
 * Center-pane modes — Ask is the marquee surface; Explore lives here too
 * as an internal toggle so the Search/Symbols/Bridges flows aren't lost.
 * (The deeper "inline filter chips merge into Ask" rewrite is a follow-up.)
 */
type CenterMode = "ask" | "explore";

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
  // Initialize density preference (sets data-density on <html>).
  useDensity();
  const { toast } = useToast();

  // ── Workspace state ────────────────────────────────────────────────────────
  const [centerMode, setCenterMode] = useState<CenterMode>("ask");
  const [exploreMode, setExploreMode] = useState<ExploreMode | undefined>(
    undefined,
  );
  const [showOnboarding, setShowOnboarding] = useState(false);

  // Modals / drawers
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [logsOpen, setLogsOpen] = useState(false);
  const [sessionOpen, setSessionOpen] = useState(false);
  const [indexingOpen, setIndexingOpen] = useState(false);
  const [manageProjectsOpen, setManageProjectsOpen] = useState(false);

  // Investigation state — single source of truth, lifted here so the
  // CorpusRail (list/select), AskView (record query), and SourcePane
  // (pinned ids) all read/write the same store snapshot.
  const investigations = useInvestigations(activeCorpusId);

  const gPending = useRef(false);
  const gTimer = useRef<number | null>(null);

  useEffect(() => {
    invoke<boolean>("should_show_onboarding").then(setShowOnboarding);
  }, []);

  useEffect(() => {
    const unlistenNav = listen<string>("navigate", (event) => {
      const target = event.payload;
      if (target === "ask") setCenterMode("ask");
      else if (target === "explore") setCenterMode("explore");
      else if (target === "settings") setSettingsOpen(true);
      else if (target === "sessions") setSessionOpen(true);
    });
    const unlistenSelect = listen<string>("select-corpus", (event) => {
      if (typeof event.payload === "string") {
        setActiveCorpusId(event.payload);
      }
    });
    function onWindowNavigate(e: Event) {
      const detail = (e as CustomEvent).detail;
      if (detail === "ask") setCenterMode("ask");
      else if (detail === "explore") setCenterMode("explore");
      else if (detail === "settings") setSettingsOpen(true);
      else if (detail === "sessions") setSessionOpen(true);
    }
    window.addEventListener("ministr-navigate", onWindowNavigate);
    return () => {
      unlistenNav.then((fn) => fn());
      unlistenSelect.then((fn) => fn());
      window.removeEventListener("ministr-navigate", onWindowNavigate);
    };
  }, [setActiveCorpusId]);

  // Global keyboard shortcuts. Many old nav targets retire — projects/
  // sessions/logs are no longer top-level routes; map them to drawers.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null;
      const typing =
        target?.tagName === "INPUT" ||
        target?.tagName === "TEXTAREA" ||
        target?.isContentEditable;

      const result = matchShortcut(e, gPending.current);

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
        // Topmost-overlay-first wins. Modals beat drawers beat the panel.
        if (paletteOpen) {
          e.preventDefault();
          e.stopImmediatePropagation();
          setPaletteOpen(false);
        } else if (shortcutsOpen) {
          e.preventDefault();
          e.stopImmediatePropagation();
          setShortcutsOpen(false);
        } else if (settingsOpen) {
          e.preventDefault();
          e.stopImmediatePropagation();
          setSettingsOpen(false);
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
            // Rail is permanent in the workspace shell — reuse the shortcut
            // to toggle the source pane visibility instead. (TODO follow-up:
            // route through WorkspaceShell.)
            return;
          case "nav:ask":
            setCenterMode("ask");
            return;
          case "nav:explore":
            setCenterMode("explore");
            setExploreMode(undefined);
            return;
          case "nav:projects":
            setManageProjectsOpen(true);
            return;
          case "nav:sessions":
            setSessionOpen(true);
            return;
          case "nav:logs":
            setLogsOpen(true);
            return;
          case "nav:settings":
            setSettingsOpen(true);
            return;
          case "toggle:palette":
            setPaletteOpen((o) => !o);
            return;
        }
      }
    }

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [paletteOpen, shortcutsOpen, settingsOpen]);

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
    if (c) toast("Corpus", { detail: corpusLabel(c), tone: "info" });
  }

  function onThemeChange(t: "system" | "dark" | "light") {
    setTheme(t);
    toast("THEME", { detail: t.toUpperCase(), tone: "info" });
  }

  // First-run onboarding — full-screen takeover.
  if (showOnboarding) {
    return <Onboarding onDismiss={() => setShowOnboarding(false)} />;
  }

  return (
    <>
      <WorkspaceShell
        banner={
          error ? (
            <div className="flex items-center gap-2 border-b-2 border-danger bg-surface px-5 py-2 text-xs font-mono tracking-[0.05em] text-danger shrink-0">
              <AlertTriangle className="h-3.5 w-3.5 shrink-0" strokeWidth={2.5} />
              <span>{error}</span>
            </div>
          ) : null
        }
        rail={
          <CorpusRail
            status={status}
            activeCorpusId={activeCorpusId}
            onSelectCorpus={onSelectCorpus}
            onAddProject={openAddProject}
            onManageProjects={() => setManageProjectsOpen(true)}
            investigations={investigations.investigations}
            activeInvestigationId={investigations.active?.id ?? null}
            onSelectInvestigation={(id) => investigations.setActive(id)}
            onNewInvestigation={() => investigations.create()}
            onCloseInvestigation={(id) => investigations.close(id)}
          />
        }
        center={
          <CenterPane
            status={status}
            error={error}
            mode={centerMode}
            onModeChange={setCenterMode}
            activeCorpusId={activeCorpusId}
            setActiveCorpusId={setActiveCorpusId}
            exploreMode={exploreMode}
          />
        }
        source={
          <SourcePane
            corpusId={activeCorpusId}
            pinnedSourceIds={investigations.pinnedSourceIds}
            onUnpin={investigations.unpin}
            onClear={investigations.clearPins}
          />
        }
        statusBar={
          <StatusBar
            status={status}
            error={error}
            activeCorpus={activeCorpus}
            onOpenLogs={() => setLogsOpen(true)}
            onOpenSession={() => setSessionOpen(true)}
            onOpenIndexing={() => setIndexingOpen(true)}
            onOpenPalette={() => setPaletteOpen(true)}
            onOpenSettings={() => setSettingsOpen(true)}
          />
        }
      />

      {/* ── Modals & drawers ────────────────────────────────────────────── */}

      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        status={status}
        activeCorpusId={activeCorpusId}
        onNavigate={(t) => {
          if (t === "ask" || t === "explore") setCenterMode(t);
          else if (t === "projects") setManageProjectsOpen(true);
          else if (t === "sessions") setSessionOpen(true);
          else if (t === "settings") setSettingsOpen(true);
        }}
        onNavigateExplore={(mode) => {
          setCenterMode("explore");
          setExploreMode(mode);
        }}
        onOpenDiagnostics={(target) => {
          setSettingsOpen(true);
          requestAnimationFrame(() => {
            window.dispatchEvent(
              new CustomEvent("ministr-settings-scroll", { detail: target }),
            );
          });
        }}
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

      {/* Settings — modal shell hosting the existing Settings component. */}
      {settingsOpen && status && (
        <SettingsModal
          onClose={() => setSettingsOpen(false)}
          status={status}
          theme={theme}
          onThemeChange={onThemeChange}
          onShowOnboarding={() => {
            setSettingsOpen(false);
            setShowOnboarding(true);
          }}
          onRefresh={refresh}
          onOpenLogs={() => {
            setSettingsOpen(false);
            setLogsOpen(true);
          }}
        />
      )}

      <Drawer
        open={logsOpen}
        onClose={() => setLogsOpen(false)}
        title="Daemon log"
      >
        <div className="h-full">
          <LogViewer />
        </div>
      </Drawer>

      <Drawer
        open={sessionOpen}
        onClose={() => setSessionOpen(false)}
        title="Session vitals"
      >
        {status && (
          <div className="p-4">
            <SessionDashboard status={status} />
          </div>
        )}
      </Drawer>

      <Drawer
        open={indexingOpen}
        onClose={() => setIndexingOpen(false)}
        title="Indexing"
        heightVh={45}
      >
        <div className="p-4">
          {status && status.corpora.some((c) => c.status.state === "indexing") ? (
            <IndexingDetail status={status} />
          ) : (
            <p className="font-serif text-sm italic text-text-dim">
              No indexing in flight.
            </p>
          )}
        </div>
      </Drawer>

      <Drawer
        open={manageProjectsOpen}
        onClose={() => setManageProjectsOpen(false)}
        title="Manage projects"
        heightVh={75}
      >
        {status && (
          <div className="p-5">
            <ProjectList
              corpora={status.corpora}
              onRefresh={refresh}
              onSelect={(id) => {
                setActiveCorpusId(id);
                setManageProjectsOpen(false);
              }}
              selectedId={activeCorpusId}
            />
          </div>
        )}
      </Drawer>

      {/* Universal entity-detail drawer — keeps existing drill-deeper UX
          (breadcrumbs, related symbols) while the SourcePane handles
          the persistent pinned stack. */}
      <EntityPanel />
    </>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Center pane

function CenterPane({
  status,
  error,
  mode,
  onModeChange,
  activeCorpusId,
  setActiveCorpusId,
  exploreMode,
}: {
  status: import("./lib/types").DaemonStatus | null;
  error: string | null;
  mode: CenterMode;
  onModeChange: (m: CenterMode) => void;
  activeCorpusId: string | null;
  setActiveCorpusId: (id: string | null) => void;
  exploreMode: ExploreMode | undefined;
}) {
  if (!status) {
    return <ConnectingState error={error ?? null} />;
  }

  return (
    <div className="flex flex-col h-full min-h-0">
      <CenterModeStrip mode={mode} onChange={onModeChange} />
      <div className="flex-1 min-h-0 overflow-y-auto p-5">
        {mode === "ask" ? (
          <AskView
            status={status}
            activeCorpusId={activeCorpusId}
            setActiveCorpusId={setActiveCorpusId}
          />
        ) : (
          <ExploreView
            status={status}
            activeCorpusId={activeCorpusId}
            setActiveCorpusId={setActiveCorpusId}
            initialMode={exploreMode}
          />
        )}
      </div>
    </div>
  );
}

function CenterModeStrip({
  mode,
  onChange,
}: {
  mode: CenterMode;
  onChange: (m: CenterMode) => void;
}) {
  const items: { key: CenterMode; label: string; icon: typeof BrutalAsk }[] = [
    { key: "ask", label: "Ask", icon: BrutalAsk },
    { key: "explore", label: "Explore", icon: BrutalExplore },
  ];
  return (
    <div className="flex items-center gap-0 border-b-2 border-border bg-surface px-3 py-1.5 shrink-0">
      {items.map(({ key, label, icon: Icon }) => {
        const active = key === mode;
        return (
          <button
            key={key}
            onClick={() => onChange(key)}
            className={cn(
              "inline-flex items-center gap-1.5 px-3 py-1 cursor-pointer transition-none -ml-[1px] first:ml-0 rounded-sm",
              "border border-border-soft bg-surface",
              active
                ? "border-accent bg-surface-overlay text-text z-10 relative"
                : "text-text-muted hover:bg-surface-overlay hover:text-text",
            )}
          >
            <Icon className="h-3.5 w-3.5" />
            <span className="font-mono text-xs font-semibold">{label}</span>
          </button>
        );
      })}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Misc

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

function IndexingDetail({
  status,
}: {
  status: import("./lib/types").DaemonStatus;
}) {
  const indexing = status.corpora.filter((c) => c.status.state === "indexing");
  return (
    <ul className="space-y-3">
      {indexing.map((c) => {
        const s = c.status;
        if (s.state !== "indexing") return null;
        const pct = s.files_total > 0
          ? Math.round((s.files_done / s.files_total) * 100)
          : 0;
        return (
          <li
            key={c.id}
            className="border-2 border-border bg-surface px-4 py-3"
          >
            <div className="flex items-center justify-between gap-3 mb-2">
              <span className="font-mono text-sm font-bold text-text">
                {corpusLabel(c)}
              </span>
              <span className="font-mono text-xs tabular-nums text-text-muted">
                {s.files_done.toLocaleString()} / {s.files_total.toLocaleString()} files · {pct}%
              </span>
            </div>
            <div className="h-2 bg-surface-overlay border-2 border-border">
              <div
                className="h-full bg-accent-live"
                style={{ width: `${pct}%` }}
              />
            </div>
          </li>
        );
      })}
    </ul>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Settings modal — wraps the existing Settings component in modal chrome.

function SettingsModal({
  onClose,
  status,
  theme,
  onThemeChange,
  onShowOnboarding,
  onRefresh,
  onOpenLogs,
}: {
  onClose: () => void;
  status: import("./lib/types").DaemonStatus;
  theme: "system" | "dark" | "light";
  onThemeChange: (t: "system" | "dark" | "light") => void;
  onShowOnboarding: () => void;
  onRefresh: () => void;
  onOpenLogs: () => void;
}) {
  return (
    <>
      <div
        className="fixed inset-0 z-[1200] bg-black/40"
        onClick={onClose}
        aria-hidden="true"
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Settings"
        className={cn(
          "fixed top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2",
          "z-[1201] bg-surface border-2 border-border shadow-lg",
          "w-[clamp(640px,80vw,920px)] max-h-[85vh] overflow-hidden",
          "flex flex-col",
        )}
      >
        <header className="flex items-center justify-between gap-3 border-b-2 border-border bg-surface-overlay px-4 py-2.5 shrink-0">
          <h2 className="font-mono text-sm font-bold uppercase tracking-[0.05em] text-text">
            Settings
          </h2>
          <button
            onClick={onClose}
            aria-label="Close settings"
            title="Close · Esc"
            className={cn(
              "grid h-7 w-7 shrink-0 place-items-center cursor-pointer",
              "border border-border bg-surface text-text-muted",
              "hover:text-text hover:border-border-hover transition-none rounded-sm",
            )}
          >
            ×
          </button>
        </header>
        <div className="flex-1 min-h-0 overflow-y-auto p-5">
          <Settings
            status={status}
            theme={theme}
            onThemeChange={onThemeChange}
            onShowOnboarding={onShowOnboarding}
            onRefresh={onRefresh}
            onOpenLogs={onOpenLogs}
          />
        </div>
      </div>
    </>
  );
}
