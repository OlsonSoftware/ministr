import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AlertTriangle } from "lucide-react";
import { useDaemonStatus } from "./hooks/useDaemonStatus";
import { useTheme } from "./hooks/useTheme";
import { useCorpusContext } from "./hooks/useCorpusContext";
import { useDensity } from "./hooks/usePreferences";
import { Onboarding } from "./components/Onboarding";
import { AskSurface } from "./components/surfaces/ask/AskSurface";
import { ShortcutSheet } from "./components/ShortcutSheet";
import { ToastProvider, useToast } from "./components/shell/ToastTray";
import { EntityPanelProvider } from "./hooks/useEntityPanel";
import { EntityPanel } from "./components/EntityPanel";
import { Sidebar, type SurfaceId } from "./components/chrome/Sidebar";
import { TopBar } from "./components/chrome/TopBar";
import { ProjectsSurface } from "./components/surfaces/ProjectsSurface";
import { SettingsSurface } from "./components/surfaces/SettingsSurface";
import { corpusLabel } from "./lib/corpus";
import {
  matchShortcut,
  firesWhileTyping,
  type ShortcutAction,
} from "./lib/shortcuts";

/**
 * App shell — three top-level surfaces (Ask / Projects / Settings) wired
 * around a persistent project picker in the top bar.
 *
 * The previous workspace shell (rail + center modes + source pane + status
 * bar + four drawers) is replaced by a flat surface switcher. Power-user
 * features (Sessions / Logs / Activity / Bridges / Query playground) move
 * into Settings → Developer Tools (M4).
 */
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
  const { activeCorpusId, setActiveCorpusId } = useCorpusContext(status);
  // Initialize density preference (sets data-density on <html>).
  useDensity();
  const { toast } = useToast();

  const [surface, setSurface] = useState<SurfaceId>("ask");
  const [showOnboarding, setShowOnboarding] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);

  const gPending = useRef(false);
  const gTimer = useRef<number | null>(null);

  useEffect(() => {
    invoke<boolean>("should_show_onboarding").then(setShowOnboarding);
  }, []);

  // External navigation events from tray menu / deep links.
  useEffect(() => {
    const unlistenNav = listen<string>("navigate", (event) => {
      const target = event.payload;
      if (target === "ask" || target === "projects" || target === "settings") {
        setSurface(target);
      } else if (target === "sessions" || target === "explore") {
        // Deprecated targets — settle on Settings → Developer Tools (M4).
        setSurface("settings");
      }
    });
    const unlistenSelect = listen<string>("select-corpus", (event) => {
      if (typeof event.payload === "string") {
        setActiveCorpusId(event.payload);
      }
    });
    function onWindowNavigate(e: Event) {
      const detail = (e as CustomEvent).detail;
      if (detail === "ask" || detail === "projects" || detail === "settings") {
        setSurface(detail);
      }
    }
    window.addEventListener("ministr-navigate", onWindowNavigate);
    return () => {
      unlistenNav.then((fn) => fn());
      unlistenSelect.then((fn) => fn());
      window.removeEventListener("ministr-navigate", onWindowNavigate);
    };
  }, [setActiveCorpusId]);

  // Global keyboard shortcuts. Pruned to the three surfaces that exist.
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
        if (shortcutsOpen) {
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
          case "nav:ask":
            setSurface("ask");
            return;
          case "nav:projects":
            setSurface("projects");
            return;
          case "nav:settings":
            setSurface("settings");
            return;
          case "toggle:palette":
            // Command palette is offline in M1; ⌘K is reserved for M4
            // when the new palette ships. No-op here so we don't trap
            // the keystroke without affordance.
            return;
        }
      }
    }

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [shortcutsOpen]);

  const openAddProject = useCallback(async () => {
    try {
      await invoke("add_project_dialog");
      refresh();
      toast("PROJECT ADDED", { tone: "success" });
    } catch {
      /* user cancelled */
    }
  }, [refresh, toast]);

  const onSelectCorpus = useCallback(
    (id: string) => {
      const c = status?.corpora.find((x) => x.id === id);
      setActiveCorpusId(id);
      if (c) toast("Project", { detail: corpusLabel(c), tone: "info" });
    },
    [status, setActiveCorpusId, toast],
  );

  const onThemeChange = useCallback(
    (t: "system" | "dark" | "light") => {
      setTheme(t);
      toast("THEME", { detail: t.toUpperCase(), tone: "info" });
    },
    [setTheme, toast],
  );

  const onOpenLogs = useCallback(async () => {
    if (status?.log_path) {
      try {
        await invoke("open_path", { path: status.log_path });
      } catch {
        /* ignore */
      }
    }
  }, [status]);

  const corpora = status?.corpora ?? [];
  const hasCorpora = corpora.length > 0;

  // Smart default: if we land with no projects, route the user to Projects
  // so the empty state's "Add" CTA is the obvious next step. Declared
  // before the onboarding early return so the hook order stays stable.
  useEffect(() => {
    if (status && !hasCorpora && surface === "ask") {
      setSurface("projects");
    }
    // Only run when projects appear/disappear, not on every surface change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasCorpora, status]);

  // First-run onboarding — full-screen takeover.
  if (showOnboarding) {
    return <Onboarding onDismiss={() => setShowOnboarding(false)} />;
  }

  return (
    <>
      <div className="flex flex-col h-screen min-h-0 bg-bg">
        <TopBar
          status={status}
          error={error}
          corpora={corpora}
          activeCorpusId={activeCorpusId}
          onSelectCorpus={onSelectCorpus}
          onAddProject={openAddProject}
          onOpenLogs={onOpenLogs}
        />

        {error && (
          <div className="flex items-center gap-2 border-b-2 border-danger bg-surface px-5 py-2 text-xs font-mono tracking-[0.05em] text-danger shrink-0">
            <AlertTriangle className="h-3.5 w-3.5 shrink-0" strokeWidth={2.5} />
            <span>{error}</span>
          </div>
        )}

        <div className="flex flex-1 min-h-0">
          <Sidebar active={surface} onSelect={setSurface} />

          <main className="flex-1 min-w-0 min-h-0 bg-bg" role="main">
            {!status ? (
              <ConnectingState error={error} />
            ) : (
              <SurfaceBody
                surface={surface}
                status={status}
                activeCorpusId={activeCorpusId}
                setActiveCorpusId={setActiveCorpusId}
                onSelectCorpus={onSelectCorpus}
                onRefresh={refresh}
                theme={theme}
                onThemeChange={onThemeChange}
                onShowOnboarding={() => setShowOnboarding(true)}
                onOpenLogs={onOpenLogs}
              />
            )}
          </main>
        </div>
      </div>

      <ShortcutSheet
        open={shortcutsOpen}
        onClose={() => setShortcutsOpen(false)}
      />

      <EntityPanel />
    </>
  );
}

function SurfaceBody({
  surface,
  status,
  activeCorpusId,
  setActiveCorpusId,
  onSelectCorpus,
  onRefresh,
  theme,
  onThemeChange,
  onShowOnboarding,
  onOpenLogs,
}: {
  surface: SurfaceId;
  status: import("./lib/types").DaemonStatus;
  activeCorpusId: string | null;
  setActiveCorpusId: (id: string | null) => void;
  onSelectCorpus: (id: string) => void;
  onRefresh: () => void;
  theme: "system" | "dark" | "light";
  onThemeChange: (t: "system" | "dark" | "light") => void;
  onShowOnboarding: () => void;
  onOpenLogs: () => void;
}) {
  if (surface === "ask") {
    return (
      <div className="h-full overflow-hidden p-5">
        <AskSurface status={status} activeCorpusId={activeCorpusId} />
      </div>
    );
  }

  if (surface === "projects") {
    return (
      <ProjectsSurface
        corpora={status.corpora}
        activeCorpusId={activeCorpusId}
        onSelectCorpus={onSelectCorpus}
        onRefresh={onRefresh}
      />
    );
  }

  return (
    <SettingsSurface
      status={status}
      activeCorpusId={activeCorpusId}
      setActiveCorpusId={setActiveCorpusId}
      theme={theme}
      onThemeChange={onThemeChange}
      onShowOnboarding={onShowOnboarding}
      onRefresh={onRefresh}
      onOpenLogs={onOpenLogs}
    />
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
