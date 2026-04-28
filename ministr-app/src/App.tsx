import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Radio,
  Users,
  Search,
  ScrollText,
  Settings as SettingsIcon,
  CircleDot,
  Cpu,
  Sparkles,
  TreePine,
  GitBranch,
  FolderKanban,
  Compass,
  ChevronDown,
  Command,
  Keyboard,
  AlertTriangle,
} from "lucide-react";
import { useDaemonStatus } from "./hooks/useDaemonStatus";
import { useTheme } from "./hooks/useTheme";
import { Overview } from "./components/Overview";
import { ProjectList } from "./components/ProjectList";
import { ProjectDetail } from "./components/ProjectDetail";
import { Settings } from "./components/Settings";
import { LogViewer } from "./components/LogViewer";
import { Onboarding } from "./components/Onboarding";
import { SessionDashboard } from "./components/SessionDashboard";
import { QueryPlayground } from "./components/QueryPlayground";
import { CorpusTreemap } from "./components/CorpusTreemap";
import { SymbolGraph } from "./components/SymbolGraph";
import { ContextSimulator } from "./components/ContextSimulator";
import { CommandPalette } from "./components/CommandPalette";
import { ShortcutSheet } from "./components/ShortcutSheet";
import { Badge } from "./components/ui/badge";
import { StatusDot } from "./components/ui/status-dot";
import { cn } from "./lib/utils";

type Tab =
  | "overview"
  | "sessions"
  | "search"
  | "treemap"
  | "symbols"
  | "simulator"
  | "logs"
  | "settings"
  | "projects";

const VALID_TABS: Tab[] = [
  "overview",
  "sessions",
  "search",
  "treemap",
  "symbols",
  "simulator",
  "logs",
  "settings",
  "projects",
];

export function App() {
  const { status, error, refresh } = useDaemonStatus();
  const { theme, setTheme } = useTheme();
  const [tab, setTab] = useState<Tab>("overview");
  const [showOnboarding, setShowOnboarding] = useState(false);
  const [selectedCorpusId, setSelectedCorpusId] = useState<string | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [railCollapsed, setRailCollapsed] = useState(false);
  const [exploreOpen, setExploreOpen] = useState(false);
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
        setSelectedCorpusId(event.payload);
      }
    });
    return () => {
      unlistenNav.then((fn) => fn());
      unlistenSelect.then((fn) => fn());
    };
  }, []);

  // Global keyboard shortcuts
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null;
      const typing =
        target?.tagName === "INPUT" ||
        target?.tagName === "TEXTAREA" ||
        target?.isContentEditable;

      // ⌘K / Ctrl+K — always available
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
        return;
      }

      if (typing) return;

      if (e.key === "?" || (e.key === "/" && e.shiftKey)) {
        e.preventDefault();
        setShortcutsOpen((o) => !o);
        return;
      }

      if (e.key === "\\") {
        e.preventDefault();
        setRailCollapsed((c) => !c);
        return;
      }

      if (e.key === "Escape") {
        if (paletteOpen) setPaletteOpen(false);
        else if (shortcutsOpen) setShortcutsOpen(false);
        return;
      }

      // g <letter> shortcuts
      if (gPending.current) {
        gPending.current = false;
        if (gTimer.current !== null) clearTimeout(gTimer.current);
        const map: Record<string, Tab> = {
          o: "overview",
          s: "sessions",
          q: "search",
          p: "projects",
          l: "logs",
          ",": "settings",
        };
        const target = map[e.key.toLowerCase()];
        if (target) {
          e.preventDefault();
          setTab(target);
        }
        return;
      }
      if (e.key === "g") {
        gPending.current = true;
        gTimer.current = window.setTimeout(() => {
          gPending.current = false;
        }, 900);
      }
    }

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [paletteOpen, shortcutsOpen]);

  const selectedCorpus = status?.corpora.find((c) => c.id === selectedCorpusId);

  const openAddProject = useCallback(async () => {
    try {
      await invoke("add_project_dialog");
      refresh();
    } catch {
      /* ignore */
    }
  }, [refresh]);

  if (showOnboarding) {
    return <Onboarding onDismiss={() => setShowOnboarding(false)} />;
  }

  return (
    <div className="flex h-screen flex-col bg-bg text-text">
      <TopBar
        status={status}
        onPaletteOpen={() => setPaletteOpen(true)}
        onShortcutsOpen={() => setShortcutsOpen(true)}
      />

      {error && (
        <div className="flex items-center gap-2 border-b border-danger/30 bg-danger/5 px-5 py-2 text-xs text-danger shrink-0">
          <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      <div className="flex flex-1 min-h-0">
        <Rail
          tab={tab}
          onSelect={setTab}
          collapsed={railCollapsed}
          exploreOpen={exploreOpen}
          onExploreToggle={() => setExploreOpen((v) => !v)}
        />

        <main className="flex-1 overflow-y-auto p-5">
          {!status ? (
            <ConnectingState error={error ?? null} />
          ) : tab === "overview" ? (
            <Overview
              status={status}
              selectedCorpusId={selectedCorpusId}
              onSelectCorpus={setSelectedCorpusId}
              onOpenProjects={() => setTab("projects")}
              onOpenSessions={() => setTab("sessions")}
              onAddProject={openAddProject}
              onRefresh={refresh}
            />
          ) : tab === "projects" ? (
            <div className="flex gap-4 h-full ministr-fade-in">
              <div
                className={cn(
                  "flex-1 min-w-0",
                  selectedCorpus && "max-w-[55%]",
                )}
              >
                <ProjectList
                  corpora={status.corpora}
                  onRefresh={refresh}
                  onSelect={setSelectedCorpusId}
                  selectedId={selectedCorpusId}
                />
              </div>
              {selectedCorpus && (
                <div className="flex-1 min-w-0 hidden md:block">
                  <ProjectDetail corpus={selectedCorpus} status={status} />
                </div>
              )}
            </div>
          ) : tab === "sessions" ? (
            <SessionDashboard status={status} />
          ) : tab === "search" ? (
            <QueryPlayground status={status} />
          ) : tab === "treemap" ? (
            <CorpusTreemap status={status} />
          ) : tab === "symbols" ? (
            <SymbolGraph status={status} />
          ) : tab === "simulator" ? (
            <ContextSimulator />
          ) : tab === "logs" ? (
            <LogViewer />
          ) : (
            <Settings
              status={status}
              theme={theme}
              onThemeChange={setTheme}
              onShowOnboarding={() => setShowOnboarding(true)}
              onRefresh={refresh}
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
        onSelectCorpus={setSelectedCorpusId}
        onShowShortcuts={() => setShortcutsOpen(true)}
        onThemeChange={setTheme}
        onRefresh={refresh}
      />
      <ShortcutSheet
        open={shortcutsOpen}
        onClose={() => setShortcutsOpen(false)}
      />
    </div>
  );
}

function TopBar({
  status,
  onPaletteOpen,
  onShortcutsOpen,
}: {
  status: import("./lib/types").DaemonStatus | null;
  onPaletteOpen: () => void;
  onShortcutsOpen: () => void;
}) {
  return (
    <header className="flex items-center justify-between gap-4 border-b border-border/70 bg-surface/50 backdrop-blur-sm px-5 py-2.5 shrink-0">
      <div className="flex items-center gap-3">
        <Logo />
        <div
          className="flex items-center gap-2"
          title={status ? `ministr v${status.version}` : "ministr"}
        >
          <span className="ministr-wordmark">ministr</span>
          <StatusDot
            tone={status ? "success" : "muted"}
            pulse={status ? "live" : "off"}
          />
        </div>
      </div>

      {status && (
        <div className="flex items-center gap-2">
          <StatChip
            icon={<Sparkles className="h-3 w-3" />}
            label={status.model.replace("all-MiniLM-", "MiniLM-")}
            sub={`${status.model_dimension}d`}
          />
          <StatChip
            icon={<Cpu className="h-3 w-3" />}
            label={`${status.memory_mb.toFixed(0)} MB`}
          />
          {status.total_sessions > 0 && (
            <Badge variant="success" dot>
              {status.total_sessions} active
            </Badge>
          )}
        </div>
      )}

      <div className="flex items-center gap-1">
        <button
          onClick={onPaletteOpen}
          title="Command palette (⌘K)"
          className="inline-flex items-center gap-1.5 rounded-md border border-border/70 bg-surface-raised/50 pl-2.5 pr-1.5 py-1 text-xs text-text-muted hover:text-text hover:border-border-hover cursor-pointer transition-all"
        >
          <Command className="h-3 w-3" />
          <span>Search</span>
          <kbd className="ml-1 rounded border border-border/60 bg-surface-overlay px-1 py-0 text-[10px] font-mono text-text-dim">
            ⌘K
          </kbd>
        </button>
        <button
          onClick={onShortcutsOpen}
          title="Shortcuts (?)"
          className="grid h-7 w-7 place-items-center rounded-md border border-transparent text-text-dim hover:text-text hover:bg-surface-overlay/60 cursor-pointer"
        >
          <Keyboard className="h-3.5 w-3.5" />
        </button>
      </div>
    </header>
  );
}

function Logo() {
  return (
    <div className="grid h-7 w-7 place-items-center rounded-lg bg-accent text-[var(--color-accent-fg-on)]">
      <CircleDot className="h-4 w-4" strokeWidth={2.5} />
    </div>
  );
}

function StatChip({
  icon,
  label,
  sub,
}: {
  icon: React.ReactNode;
  label: string;
  sub?: string;
}) {
  return (
    <span className="inline-flex items-center gap-1.5 rounded-md border border-border/60 bg-surface-overlay/40 px-2 py-1 text-[11px] font-medium text-text-muted">
      <span className="text-text-dim">{icon}</span>
      <span>{label}</span>
      {sub && <span className="font-mono text-text-dim">{sub}</span>}
    </span>
  );
}

function Rail({
  tab,
  onSelect,
  collapsed,
  exploreOpen,
  onExploreToggle,
}: {
  tab: Tab;
  onSelect: (t: Tab) => void;
  collapsed: boolean;
  exploreOpen: boolean;
  onExploreToggle: () => void;
}) {
  if (collapsed) return null;

  return (
    <nav className="hidden sm:flex flex-col w-14 border-r border-border/70 bg-surface/30 py-3 items-center gap-0.5 shrink-0">
      <RailItem
        icon={Radio}
        active={tab === "overview"}
        label="Overview"
        onClick={() => onSelect("overview")}
      />
      <RailItem
        icon={Users}
        active={tab === "sessions"}
        label="Sessions"
        onClick={() => onSelect("sessions")}
      />
      <RailItem
        icon={Search}
        active={tab === "search"}
        label="Search"
        onClick={() => onSelect("search")}
      />
      <RailItem
        icon={FolderKanban}
        active={tab === "projects"}
        label="Projects"
        onClick={() => onSelect("projects")}
      />

      <div className="relative w-full flex flex-col items-center">
        <RailItem
          icon={Compass}
          active={
            tab === "treemap" || tab === "symbols" || tab === "simulator"
          }
          label="Explore"
          onClick={onExploreToggle}
          trailing={
            <ChevronDown
              className={cn(
                "absolute right-1.5 top-2.5 h-2.5 w-2.5 text-text-dim transition-transform",
                exploreOpen && "rotate-180",
              )}
            />
          }
        />
        {exploreOpen && (
          <div className="flex flex-col items-center gap-0.5">
            <RailItem
              icon={TreePine}
              active={tab === "treemap"}
              label="Treemap"
              onClick={() => onSelect("treemap")}
              size="sm"
            />
            <RailItem
              icon={GitBranch}
              active={tab === "symbols"}
              label="Symbols"
              onClick={() => onSelect("symbols")}
              size="sm"
            />
            <RailItem
              icon={Cpu}
              active={tab === "simulator"}
              label="Simulator"
              onClick={() => onSelect("simulator")}
              size="sm"
            />
          </div>
        )}
      </div>

      <RailItem
        icon={ScrollText}
        active={tab === "logs"}
        label="Logs"
        onClick={() => onSelect("logs")}
      />
      <div className="flex-1" />
      <RailItem
        icon={SettingsIcon}
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
  size = "md",
  trailing,
}: {
  icon: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  active: boolean;
  onClick: () => void;
  label: string;
  size?: "sm" | "md";
  trailing?: React.ReactNode;
}) {
  const dim = size === "sm" ? "h-7 w-7" : "h-9 w-9";
  const iconSize = size === "sm" ? "h-4 w-4" : "h-[18px] w-[18px]";
  return (
    <button
      onClick={onClick}
      title={label}
      aria-label={label}
      className={cn(
        "relative grid place-items-center rounded-lg transition-all duration-150 cursor-pointer",
        dim,
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-accent-ring)]",
        active
          ? "bg-[var(--color-accent-soft)] text-accent"
          : "text-text-dim hover:text-text hover:bg-surface-overlay/70",
      )}
    >
      {active && (
        <span className="absolute left-[-9px] top-1/2 h-5 w-0.5 -translate-y-1/2 rounded-full bg-accent" />
      )}
      <Icon className={iconSize} strokeWidth={active ? 2.25 : 2} />
      {trailing}
    </button>
  );
}

function ConnectingState({ error }: { error: string | null }) {
  return (
    <div className="flex flex-col items-center justify-center h-full gap-4 ministr-fade-in">
      <div className="relative">
        <div className="ministr-spin h-10 w-10 rounded-full border-2 border-border border-t-accent" />
        <CircleDot className="absolute inset-0 m-auto h-4 w-4 text-accent ministr-pulse" />
      </div>
      <div className="text-center">
        <p className="text-sm font-medium text-text">Connecting to daemon…</p>
      </div>
      {error && (
        <p className="max-w-md text-center text-xs text-danger/80 mt-2">
          {error}
        </p>
      )}
    </div>
  );
}
