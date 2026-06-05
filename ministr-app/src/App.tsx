import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, RotateCw, FileText } from "@/components/ui/icons";
import { Button } from "./components/ui/button";
import { AnimatePresence, motion } from "motion/react";
import { useDaemonStatus } from "./hooks/useDaemonStatus";
import { useTheme } from "./hooks/useTheme";
import { useDensity } from "./hooks/usePreferences";
import { FirstRunOverlay } from "./components/onboarding/FirstRunGuide";
import { ToastProvider, useToast } from "./components/shell/ToastTray";
import { ConnectingState } from "./components/shell/ConnectingState";
import { EntityPanelProvider } from "./hooks/useEntityPanel";
import { WorkspaceProvider } from "./components/workspace/WorkspaceContext";
import { WorkspaceScreen } from "./components/workspace/WorkspaceScreen";
import { useLiveEvents } from "./lib/liveBus";

/**
 * App shell. One integrated, object-centric workspace (AAA·IA): a project/fleet
 * spine selected once + a facet switcher (Ask·Explore·Activity·Tend) sharing one
 * context. The six sibling destinations + the per-surface project re-pick are
 * gone — see components/workspace/.
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
  useDensity();
  const { toast } = useToast();

  const [showOnboarding, setShowOnboarding] = useState(false);

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

  const openAddProject = useCallback(async () => {
    try {
      // Cancelling the folder picker resolves to `null` (not an error).
      const res = await invoke<{ corpus_id: string } | null>(
        "add_project_dialog",
      );
      if (!res) return;
      refresh();
      toast("Project added", { tone: "success" });
    } catch (e) {
      toast("Couldn’t add project", { detail: String(e), tone: "danger" });
    }
  }, [refresh, toast]);

  const onThemeChange = useCallback(
    (t: "system" | "dark" | "light") => {
      setTheme(t);
      toast("Theme", { detail: t.toUpperCase(), tone: "info" });
    },
    [setTheme, toast],
  );

  const onOpenLogs = useCallback(async () => {
    if (!status?.log_path) return;
    try {
      await invoke("open_path", { path: status.log_path });
    } catch (e) {
      toast("Couldn’t open logs", { detail: String(e), tone: "danger" });
    }
  }, [status, toast]);

  return (
    <WorkspaceProvider corpora={status?.corpora ?? []}>
      <div className="flex flex-col h-screen min-h-0 bg-bg">
        <AnimatePresence>
          {error && (
            <motion.div
              initial={{ height: 0, opacity: 0 }}
              animate={{ height: "auto", opacity: 1 }}
              exit={{ height: 0, opacity: 0 }}
              className="overflow-hidden border-b border-danger/50 bg-danger/10 shrink-0"
            >
              <DaemonErrorBanner
                error={error}
                unreachable={!status}
                onRetry={refresh}
                onOpenLogs={onOpenLogs}
                hasLogPath={Boolean(status?.log_path)}
              />
            </motion.div>
          )}
        </AnimatePresence>

        <div className="flex-1 min-h-0">
          {!status ? (
            <ConnectingState error={error} />
          ) : (
            <WorkspaceScreen
              status={status}
              error={error}
              theme={theme}
              onThemeChange={onThemeChange}
              onAddProject={openAddProject}
              onOpenLogs={onOpenLogs}
              onShowOnboarding={() => setShowOnboarding(true)}
              onRefresh={refresh}
            />
          )}
        </div>

        {/* First-run guide overlays the workspace (chrome visible behind) so
            the aha moment happens IN the workspace, not in a wizard you exit. */}
        <AnimatePresence>
          {showOnboarding && status && (
            <FirstRunOverlay
              status={status}
              onRefresh={refresh}
              onDone={() => setShowOnboarding(false)}
            />
          )}
        </AnimatePresence>
      </div>
    </WorkspaceProvider>
  );
}

/**
 * The top-of-shell error band. Distinguishes a daemon we can't reach
 * (no status yet — likely stopped or still starting) from a transient
 * command failure (status present, one call errored), and gives the
 * user the two things that actually help — retry and the logs.
 */
function DaemonErrorBanner({
  error,
  unreachable,
  onRetry,
  onOpenLogs,
  hasLogPath,
}: {
  error: string;
  unreachable: boolean;
  onRetry: () => void;
  onOpenLogs: () => void;
  hasLogPath: boolean;
}) {
  const title = unreachable
    ? "Can’t reach the ministr daemon"
    : "A daemon request failed";
  return (
    <div className="flex items-center gap-3 px-5 py-2 text-danger">
      <AlertTriangle className="h-3.5 w-3.5 shrink-0" strokeWidth={2} />
      <div className="min-w-0 flex-1">
        <span className="text-xs font-sans font-medium">{title}</span>
        <span className="ml-2 text-xs font-mono text-danger/70 truncate">
          {error}
        </span>
      </div>
      <Button variant="subtle" size="sm" onClick={onRetry} className="shrink-0">
        <RotateCw className="h-3.5 w-3.5" strokeWidth={2} />
        Retry
      </Button>
      {hasLogPath && (
        <Button
          variant="ghost"
          size="sm"
          onClick={onOpenLogs}
          className="shrink-0"
        >
          <FileText className="h-3.5 w-3.5" strokeWidth={2} />
          Logs
        </Button>
      )}
    </div>
  );
}

// ConnectingState now lives in components/shell/ConnectingState.tsx (storied).
