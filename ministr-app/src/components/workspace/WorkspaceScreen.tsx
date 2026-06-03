import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { DaemonStatus, SessionDetail } from "../../lib/types";
import {
  matchShortcut,
  firesWhileTyping,
  type ShortcutAction,
} from "../../lib/shortcuts";
import { useSessions } from "../../hooks/useSessions";
import { useEntityPanel } from "../../hooks/useEntityPanel";
import { useToast } from "../shell/ToastTray";
import { ShortcutSheet } from "../ShortcutSheet";
import { CommandPalette } from "../chrome/CommandPalette";
import { EntityPanel } from "../EntityPanel";
import type { SurfaceId } from "../chrome/Sidebar";
import { AskSurface } from "../surfaces/ask/AskSurface";
import { SessionsSurface } from "../surfaces/SessionsSurface";
import { ProjectsSurface } from "../surfaces/ProjectsSurface";
import { ExploreSurface } from "../surfaces/ExploreSurface";
import { TendSurface } from "../surfaces/TendSurface";
import { AccountSettings } from "../surfaces/AccountSettings";
import { WorkspaceShell } from "./WorkspaceShell";
import { useWorkspace, type FacetId } from "./WorkspaceContext";

interface Props {
  status: DaemonStatus | null;
  error: string | null;
  theme: "system" | "dark" | "light";
  onThemeChange: (t: "system" | "dark" | "light") => void;
  onAddProject: () => void;
  onOpenLogs: () => void;
  onShowOnboarding: () => void;
  onRefresh: () => void;
}

/**
 * The live workspace, mounted inside {@link WorkspaceProvider}. It reads the
 * ONE shared spine context and maps it onto the shipped surfaces:
 *   Project spine → Ask·Explore·Activity·Tend facets, scoped to the project.
 *   Fleet spine   → the Projects collection; picking a project zooms in.
 *
 * It also owns the cross-cutting chrome (command palette, shortcut sheet,
 * entity inspector) and the keyboard shortcuts, all wired to the spine instead
 * of the retired six-destination nav.
 */
export function WorkspaceScreen({
  status,
  error,
  theme,
  onThemeChange,
  onAddProject,
  onOpenLogs,
  onShowOnboarding,
  onRefresh,
}: Props) {
  const { activeProjectId, isFleet, selectProject, selectFleet, setFacet } =
    useWorkspace();
  const { sessions } = useSessions();
  const { openEntity } = useEntityPanel();
  const { toast } = useToast();

  const [paletteOpen, setPaletteOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [accountOpen, setAccountOpen] = useState(false);
  const gPending = useRef(false);
  const gTimer = useRef<number | null>(null);

  // A facet selection from anywhere (rail-less): nav verbs map onto the spine.
  const goSurface = useCallback(
    (s: SurfaceId) => {
      switch (s) {
        case "ask":
          setFacet("ask");
          return;
        case "explore":
          setFacet("explore");
          return;
        case "sessions":
          setFacet("activity");
          return;
        case "settings":
          setFacet("tend");
          return;
        case "projects":
          selectFleet();
          return;
        case "cloud":
          // Cloud folds into the thin global Account area (cloud connection +
          // sharing live there, not in a parallel destination).
          setAccountOpen(true);
          return;
      }
    },
    [setFacet, selectFleet],
  );

  // External navigation events (tray menu / deep links) + corpus selection.
  useEffect(() => {
    const unlistenNav = listen<string>("navigate", (event) => {
      goSurface(event.payload as SurfaceId);
    });
    const unlistenSelect = listen<string>("select-corpus", (event) => {
      if (typeof event.payload === "string") selectProject(event.payload);
    });
    function onWindowNavigate(e: Event) {
      goSurface((e as CustomEvent).detail as SurfaceId);
    }
    window.addEventListener("ministr-navigate", onWindowNavigate);
    return () => {
      unlistenNav.then((fn) => fn());
      unlistenSelect.then((fn) => fn());
      window.removeEventListener("ministr-navigate", onWindowNavigate);
    };
  }, [goSurface, selectProject]);

  // Global keyboard shortcuts (g-chord + ⌘K), wired to the spine.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null;
      const typing =
        target?.tagName === "INPUT" ||
        target?.tagName === "TEXTAREA" ||
        target?.isContentEditable;

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
            setFacet("ask");
            return;
          case "nav:projects":
            selectFleet();
            return;
          case "nav:sessions":
            setFacet("activity");
            return;
          case "nav:cloud":
            setAccountOpen(true);
            return;
          case "nav:explore":
            setFacet("explore");
            return;
          case "nav:settings":
            setFacet("tend");
            return;
          case "toggle:palette":
            setPaletteOpen((o) => !o);
            return;
        }
      }
    }

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [shortcutsOpen, paletteOpen, setFacet, selectFleet]);

  const onReindexActive = useCallback(async () => {
    if (!activeProjectId) {
      toast("No active project", {
        detail: "Select a project first",
        tone: "info",
      });
      return;
    }
    try {
      await invoke("trigger_reindex", { corpusId: activeProjectId });
      toast("Re-indexing started", { tone: "info" });
      onRefresh();
    } catch (e) {
      toast("Re-index failed", { detail: String(e), tone: "danger" });
    }
  }, [activeProjectId, toast, onRefresh]);

  const onCycleTheme = useCallback(() => {
    const order = ["system", "dark", "light"] as const;
    onThemeChange(order[(order.indexOf(theme) + 1) % order.length]);
  }, [theme, onThemeChange]);

  // The live ⚡ layer is scoped to the spine: a project's agents on a project,
  // the whole fleet when zoomed out.
  const scopedSessions = isFleet
    ? sessions
    : sessions.filter((s) => s.corpus_id === activeProjectId);

  const onOpenSession = useCallback(
    (s: SessionDetail) =>
      openEntity({
        kind: "session",
        corpusId: s.corpus_id,
        sessionId: s.session_id,
        seed: s,
      }),
    [openEntity],
  );

  const renderFacet = useCallback(
    (facet: FacetId) => {
      if (!status) return null;
      const cid = activeProjectId;
      switch (facet) {
        case "ask":
          return <AskSurface status={status} activeCorpusId={cid} />;
        case "explore":
          return <ExploreSurface status={status} activeCorpusId={cid} />;
        case "activity":
          return <SessionsSurface status={status} activeCorpusId={cid} />;
        case "tend":
          return <TendSurface onRefresh={onRefresh} />;
      }
    },
    [status, activeProjectId, onRefresh],
  );

  const renderFleet = useCallback(() => {
    if (!status) return null;
    return (
      <ProjectsSurface
        corpora={status.corpora}
        activeCorpusId={activeProjectId}
        onSelectCorpus={selectProject}
        onRefresh={onRefresh}
      />
    );
  }, [status, activeProjectId, selectProject, onRefresh]);

  return (
    <>
      <WorkspaceShell
        status={status}
        error={error}
        sessions={scopedSessions}
        onOpenSession={onOpenSession}
        onAddProject={onAddProject}
        onOpenLogs={onOpenLogs}
        onOpenPalette={() => setPaletteOpen(true)}
        onOpenAccount={() => setAccountOpen(true)}
        renderFacet={renderFacet}
        renderFleet={renderFleet}
      />

      <ShortcutSheet open={shortcutsOpen} onClose={() => setShortcutsOpen(false)} />

      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        corpora={status?.corpora ?? []}
        activeCorpusId={activeProjectId}
        onNavigate={goSurface}
        onSelectCorpus={selectProject}
        onAddProject={onAddProject}
        onOpenLogs={onOpenLogs}
        onReindexActive={onReindexActive}
        onCycleTheme={onCycleTheme}
      />

      {status && (
        <AccountSettings
          open={accountOpen}
          onClose={() => setAccountOpen(false)}
          status={status}
          activeCorpusId={activeProjectId}
          theme={theme}
          onThemeChange={onThemeChange}
          onShowOnboarding={onShowOnboarding}
          onRefresh={onRefresh}
          onOpenLogs={onOpenLogs}
        />
      )}

      <EntityPanel />
    </>
  );
}
