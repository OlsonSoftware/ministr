import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AlertTriangle } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useDaemonStatus } from "./hooks/useDaemonStatus";
import { useTheme } from "./hooks/useTheme";
import { useCorpusContext } from "./hooks/useCorpusContext";
import { useDensity, useDefaultTab } from "./hooks/usePreferences";
import { Onboarding } from "./components/Onboarding";
import { AskSurface } from "./components/surfaces/ask/AskSurface";
import { SessionsSurface } from "./components/surfaces/SessionsSurface";
import { ShortcutSheet } from "./components/ShortcutSheet";
import { ToastProvider, useToast } from "./components/shell/ToastTray";
import { EntityPanelProvider } from "./hooks/useEntityPanel";
import { EntityPanel } from "./components/EntityPanel";
import { Sidebar, type SurfaceId } from "./components/chrome/Sidebar";
import { TopBar } from "./components/chrome/TopBar";
import { CommandPalette } from "./components/chrome/CommandPalette";
import { ProjectsSurface } from "./components/surfaces/ProjectsSurface";
import { SettingsSurface } from "./components/surfaces/SettingsSurface";
import { corpusLabel } from "./lib/corpus";
import { useLiveEvents } from "./lib/liveBus";
import { fade } from "./lib/motion";
import {
  matchShortcut,
  firesWhileTyping,
  type ShortcutAction,
} from "./lib/shortcuts";

/**
 * App shell — the Cockpit. Four top-level surfaces (Ask / Projects /
 * Sessions / Settings) behind a nav rail + context-aware top bar, a
 * global command palette, and a stacked entity inspector. Surface
 * switches animate; a small back/forward history backs ⌘[ / ⌘].
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
  useDensity();
  const { defaultTab } = useDefaultTab();
  const { toast } = useToast();

  // Surface + a small navigation history (back/forward).
  const [history, setHistory] = useState<SurfaceId[]>([defaultTab]);
  const [cursor, setCursor] = useState(0);
  const surface = history[cursor];

  // Refs so the keyboard/event handlers always see the latest state
  // without re-binding listeners on every navigation.
  const historyRef = useRef(history);
  const cursorRef = useRef(cursor);
  historyRef.current = history;
  cursorRef.current = cursor;

  const navigate = useCallback((next: SurfaceId) => {
    const h = historyRef.current;
    const c = cursorRef.current;
    if (h[c] === next) return;
    const nh = [...h.slice(0, c + 1), next];
    setHistory(nh);
    setCursor(nh.length - 1);
  }, []);

  const back = useCallback(() => setCursor((c) => Math.max(0, c - 1)), []);
  const forward = useCallback(
    () => setCursor((c) => Math.min(historyRef.current.length - 1, c + 1)),
    [],
  );

  const [showOnboarding, setShowOnboarding] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);

  const gPending = useRef(false);
  const gTimer = useRef<number | null>(null);

  useEffect(() => {
    invoke<boolean>("should_show_onboarding").then(setShowOnboarding);
  }, []);

  // Ambient liveness — surface session lifecycle moments as toasts.
  useLiveEvents(
    useCallback(
      (e) => {
        if (e.kind === "session-started") {
          toast("Agent connected", {
            detail: e.session.session_id.slice(0, 12),
            tone: "success",
          });
        } else if (e.kind === "session-ended") {
          toast("Session ended", {
            detail: e.sessionId.slice(0, 12),
            tone: "info",
          });
        } else if (e.kind === "pressure-critical") {
          toast("Context critical", {
            detail: e.session.session_id.slice(0, 12),
            tone: "danger",
          });
        }
      },
      [toast],
    ),
  );

  // External navigation events from tray menu / deep links.
  useEffect(() => {
    const unlistenNav = listen<string>("navigate", (event) => {
      const t = event.payload;
      if (
        t === "ask" ||
        t === "projects" ||
        t === "sessions" ||
        t === "settings"
      ) {
        navigate(t);
      } else if (t === "explore") {
        navigate("settings");
      }
    });
    const unlistenSelect = listen<string>("select-corpus", (event) => {
      if (typeof event.payload === "string") {
        setActiveCorpusId(event.payload);
      }
    });
    function onWindowNavigate(e: Event) {
      const detail = (e as CustomEvent).detail;
      if (
        detail === "ask" ||
        detail === "projects" ||
        detail === "sessions" ||
        detail === "settings"
      ) {
        navigate(detail);
      }
    }
    window.addEventListener("ministr-navigate", onWindowNavigate);
    return () => {
      unlistenNav.then((fn) => fn());
      unlistenSelect.then((fn) => fn());
      window.removeEventListener("ministr-navigate", onWindowNavigate);
    };
  }, [setActiveCorpusId, navigate]);

  // Global keyboard shortcuts.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null;
      const typing =
        target?.tagName === "INPUT" ||
        target?.tagName === "TEXTAREA" ||
        target?.isContentEditable;

      // ⌘[ / ⌘] — history back/forward (works even while typing is fine).
      if ((e.metaKey || e.ctrlKey) && (e.key === "[" || e.key === "]")) {
        e.preventDefault();
        if (e.key === "[") back();
        else forward();
        return;
      }

      const result = matchShortcut(e, gPending.current);

      if (result && result !== "_pending:g" && firesWhileTyping(result)) {
        e.preventDefault();
        dispatchShortcut(result);
        return;
      }

      if (typing) return;

      if (e.key === "Escape") {
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
          case "nav:ask":
            navigate("ask");
            return;
          case "nav:projects":
            navigate("projects");
            return;
          case "nav:sessions":
            navigate("sessions");
            return;
          case "nav:settings":
            navigate("settings");
            return;
          case "toggle:palette":
            setPaletteOpen((o) => !o);
            return;
        }
      }
    }

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [shortcutsOpen, paletteOpen, navigate, back, forward]);

  const openAddProject = useCallback(async () => {
    try {
      await invoke("add_project_dialog");
      refresh();
      toast("Project added", { tone: "success" });
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
      toast("Theme", { detail: t.toUpperCase(), tone: "info" });
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

  // Cold install with no projects → bounce to Projects so the empty
  // state's CTA is the obvious next step.
  useEffect(() => {
    if (status && !hasCorpora && surface === "ask") {
      navigate("projects");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasCorpora, status]);

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
          onOpenPalette={() => setPaletteOpen(true)}
        />

        <AnimatePresence>
          {error && (
            <motion.div
              initial={{ height: 0, opacity: 0 }}
              animate={{ height: "auto", opacity: 1 }}
              exit={{ height: 0, opacity: 0 }}
              className="flex items-center gap-2 overflow-hidden border-b border-danger/50 bg-danger/10 px-5 py-2 text-xs font-mono text-danger shrink-0"
            >
              <AlertTriangle className="h-3.5 w-3.5 shrink-0" strokeWidth={2} />
              <span>{error}</span>
            </motion.div>
          )}
        </AnimatePresence>

        <div className="flex flex-1 min-h-0">
          <Sidebar active={surface} onSelect={navigate} />

          <main className="flex-1 min-w-0 min-h-0 bg-bg" role="main">
            {!status ? (
              <ConnectingState error={error} />
            ) : (
              <AnimatePresence mode="wait">
                <motion.div
                  key={surface}
                  variants={fade}
                  initial="initial"
                  animate="animate"
                  exit="exit"
                  className="h-full"
                >
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
                </motion.div>
              </AnimatePresence>
            )}
          </main>
        </div>
      </div>

      <ShortcutSheet open={shortcutsOpen} onClose={() => setShortcutsOpen(false)} />

      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        corpora={status?.corpora ?? []}
        activeCorpusId={activeCorpusId}
        onNavigate={navigate}
        onSelectCorpus={onSelectCorpus}
        onAddProject={openAddProject}
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

  if (surface === "sessions") {
    return (
      <SessionsSurface status={status} activeCorpusId={activeCorpusId} />
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
      <div className="text-display text-text">
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
