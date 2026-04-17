import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  FolderKanban,
  Activity,
  Settings as SettingsIcon,
  ScrollText,
  Users,
  Timer,
  Search,
  TreePine,
  GitBranch,
  Cpu,
  Sparkles,
  AlertTriangle,
  CircleDot,
} from "lucide-react";
import { useDaemonStatus } from "./hooks/useDaemonStatus";
import { useTheme } from "./hooks/useTheme";
import { ProjectList } from "./components/ProjectList";
import { ProjectDetail } from "./components/ProjectDetail";
import { Settings } from "./components/Settings";
import { LogViewer } from "./components/LogViewer";
import { Onboarding } from "./components/Onboarding";
import { SessionDashboard } from "./components/SessionDashboard";
import { IngestionTimeline } from "./components/IngestionTimeline";
import { QueryPlayground } from "./components/QueryPlayground";
import { CorpusTreemap } from "./components/CorpusTreemap";
import { SymbolGraph } from "./components/SymbolGraph";
import { ContextSimulator } from "./components/ContextSimulator";
import { Badge } from "./components/ui/badge";
import { cn } from "./lib/utils";

type Tab =
  | "projects"
  | "health"
  | "sessions"
  | "ingestion"
  | "search"
  | "treemap"
  | "symbols"
  | "simulator"
  | "logs"
  | "settings";

const VALID_TABS: Tab[] = [
  "projects",
  "health",
  "sessions",
  "ingestion",
  "search",
  "treemap",
  "symbols",
  "simulator",
  "logs",
  "settings",
];

const PRIMARY_NAV: { tab: Tab; icon: typeof FolderKanban; label: string }[] = [
  { tab: "projects", icon: FolderKanban, label: "Projects" },
  { tab: "health", icon: Activity, label: "Health" },
  { tab: "sessions", icon: Users, label: "Sessions" },
  { tab: "ingestion", icon: Timer, label: "Ingestion" },
  { tab: "search", icon: Search, label: "Search" },
  { tab: "treemap", icon: TreePine, label: "Treemap" },
  { tab: "symbols", icon: GitBranch, label: "Symbols" },
  { tab: "simulator", icon: Cpu, label: "Simulator" },
  { tab: "logs", icon: ScrollText, label: "Logs" },
];

export function App() {
  const { status, error, refresh } = useDaemonStatus();
  const { theme, setTheme } = useTheme();
  const [tab, setTab] = useState<Tab>("projects");
  const [showOnboarding, setShowOnboarding] = useState(false);
  const [selectedCorpusId, setSelectedCorpusId] = useState<string | null>(null);

  useEffect(() => {
    invoke<boolean>("should_show_onboarding").then((should) => {
      setShowOnboarding(should);
    });
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

  const selectedCorpus = status?.corpora.find((c) => c.id === selectedCorpusId);

  if (showOnboarding) {
    return <Onboarding onDismiss={() => setShowOnboarding(false)} />;
  }

  return (
    <div className="flex h-screen flex-col bg-bg text-text">
      {/* Header */}
      <header className="flex items-center justify-between border-b border-border/70 bg-surface/50 backdrop-blur-sm px-5 py-3 shrink-0 shadow-[0_1px_0_rgb(0_0_0/0.02)]">
        <div className="flex items-center gap-3">
          <Logo />
          <span className="iris-wordmark">iris</span>
          {status && (
            <Badge variant="muted" className="font-mono">
              v{status.version}
            </Badge>
          )}
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
      </header>

      {/* Error banner */}
      {error && (
        <div className="flex items-center gap-2 border-b border-danger/30 bg-danger/5 px-5 py-2 text-xs text-danger shrink-0">
          <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      {/* Main content */}
      <div className="flex flex-1 min-h-0">
        {/* Sidebar */}
        <nav className="hidden sm:flex flex-col w-14 border-r border-border/70 bg-surface/30 py-3 items-center gap-0.5 shrink-0">
          {PRIMARY_NAV.map(({ tab: t, icon, label }) => (
            <NavButton
              key={t}
              icon={icon}
              active={tab === t}
              onClick={() => setTab(t)}
              label={label}
            />
          ))}
          <div className="flex-1" />
          <NavButton
            icon={SettingsIcon}
            active={tab === "settings"}
            onClick={() => setTab("settings")}
            label="Settings"
          />
        </nav>

        {/* Content area */}
        <main className="flex-1 overflow-y-auto p-5">
          {!status ? (
            <ConnectingState error={error ?? null} />
          ) : tab === "projects" ? (
            <div className="flex gap-4 h-full iris-fade-in">
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
          ) : tab === "health" ? (
            <HealthView status={status} />
          ) : tab === "sessions" ? (
            <SessionDashboard status={status} />
          ) : tab === "ingestion" ? (
            <IngestionTimeline status={status} />
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
            />
          )}
        </main>
      </div>

      {/* Bottom tabs (narrow screens) — show key tabs only */}
      <nav className="flex sm:hidden border-t border-border bg-surface/60 backdrop-blur-sm shrink-0">
        <TabButton
          icon={FolderKanban}
          label="Projects"
          active={tab === "projects"}
          onClick={() => setTab("projects")}
        />
        <TabButton
          icon={Search}
          label="Search"
          active={tab === "search"}
          onClick={() => setTab("search")}
        />
        <TabButton
          icon={Activity}
          label="Health"
          active={tab === "health"}
          onClick={() => setTab("health")}
        />
        <TabButton
          icon={ScrollText}
          label="Logs"
          active={tab === "logs"}
          onClick={() => setTab("logs")}
        />
        <TabButton
          icon={SettingsIcon}
          label="Settings"
          active={tab === "settings"}
          onClick={() => setTab("settings")}
        />
      </nav>
    </div>
  );
}

function Logo() {
  return (
    <div className="relative grid h-7 w-7 place-items-center rounded-lg bg-gradient-to-br from-accent to-[color-mix(in_srgb,var(--color-accent)_50%,#c4b5fd)] text-[var(--color-accent-fg-on)] shadow-[0_4px_14px_var(--color-accent-ring),inset_0_1px_0_rgb(255_255_255/0.2)]">
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

function NavButton({
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
        "relative grid place-items-center h-9 w-9 rounded-lg transition-all duration-150 cursor-pointer",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-accent-ring)]",
        active
          ? "bg-[var(--color-accent-soft)] text-accent"
          : "text-text-dim hover:text-text hover:bg-surface-overlay/70",
      )}
    >
      {/* Active-state vertical bar on the left edge */}
      {active && (
        <span className="absolute left-[-9px] top-1/2 h-5 w-0.5 -translate-y-1/2 rounded-full bg-accent" />
      )}
      <Icon className="h-[18px] w-[18px]" strokeWidth={active ? 2.25 : 2} />
    </button>
  );
}

function TabButton({
  icon: Icon,
  label,
  active,
  onClick,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex-1 flex flex-col items-center gap-0.5 py-2 text-[11px] transition-colors cursor-pointer",
        active ? "text-accent" : "text-text-dim",
      )}
    >
      <Icon className="h-4 w-4" />
      {label}
    </button>
  );
}

function ConnectingState({ error }: { error: string | null }) {
  return (
    <div className="flex flex-col items-center justify-center h-full gap-4 iris-fade-in">
      <div className="relative">
        <div className="iris-spin h-10 w-10 rounded-full border-2 border-border border-t-accent" />
        <CircleDot className="absolute inset-0 m-auto h-4 w-4 text-accent iris-pulse" />
      </div>
      <div className="text-center space-y-1">
        <p className="text-sm font-medium text-text">Connecting to daemon…</p>
        <p className="text-xs text-text-dim">
          Checking the Unix socket at <span className="font-mono">~/.iris/irisd.sock</span>
        </p>
      </div>
      {error && (
        <p className="max-w-md text-center text-xs text-danger/80 mt-2">
          {error}
        </p>
      )}
    </div>
  );
}

function HealthView({
  status,
}: {
  status: import("./lib/types").DaemonStatus;
}) {
  const totalFiles = status.corpora.reduce((s, c) => s + c.files_indexed, 0);
  const totalSections = status.corpora.reduce(
    (s, c) => s + c.sections_count,
    0,
  );
  const totalVectors = status.corpora.reduce(
    (s, c) => s + c.embeddings_count,
    0,
  );
  const indexing = status.corpora.filter(
    (c) => c.status.state === "indexing",
  ).length;
  const errors = status.corpora.filter(
    (c) => c.status.state === "error",
  ).length;

  return (
    <div className="space-y-6 iris-fade-in">
      <HealthSection title="Index" description="Content currently indexed across all corpora.">
        <MetricCard label="Files" value={totalFiles.toLocaleString()} />
        <MetricCard label="Sections" value={totalSections.toLocaleString()} />
        <MetricCard label="Vectors" value={totalVectors.toLocaleString()} />
        <MetricCard
          label="Memory"
          value={`${status.memory_mb.toFixed(0)} MB`}
        />
      </HealthSection>

      <HealthSection title="Runtime" description="Live activity across the daemon.">
        <MetricCard
          label="Corpora"
          value={status.corpora.length.toString()}
        />
        <MetricCard
          label="Sessions"
          value={status.total_sessions.toString()}
          highlight={status.total_sessions > 0 ? "active" : undefined}
          live={status.total_sessions > 0}
        />
        <MetricCard
          label="Indexing"
          value={indexing.toString()}
          highlight={indexing > 0 ? "warning" : undefined}
          live={indexing > 0}
        />
        <MetricCard
          label="Errors"
          value={errors.toString()}
          highlight={errors > 0 ? "danger" : undefined}
        />
      </HealthSection>

      <HealthSection title="Environment" description="Build and model metadata.">
        <MetricCard
          label="Model"
          value={status.model.replace("all-MiniLM-", "MiniLM-")}
          mono
        />
        <MetricCard label="Dimension" value={`${status.model_dimension}d`} mono />
        <MetricCard label="Uptime" value={formatUptime(status.uptime_secs)} mono />
        <MetricCard label="Version" value={`v${status.version}`} mono />
      </HealthSection>
    </div>
  );
}

function HealthSection({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <section className="space-y-3">
      <div className="flex items-baseline justify-between gap-4">
        <h2 className="text-sm font-semibold text-text">{title}</h2>
        <p className="text-xs text-text-dim">{description}</p>
      </div>
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">{children}</div>
    </section>
  );
}

function MetricCard({
  label,
  value,
  highlight,
  mono,
  live,
}: {
  label: string;
  value: string;
  highlight?: "warning" | "danger" | "active";
  mono?: boolean;
  live?: boolean;
}) {
  return (
    <div
      className={cn(
        "relative rounded-xl border border-border/70 bg-surface-raised p-4 transition-all duration-150",
        "hover:border-border-hover hover:shadow-[var(--shadow-sm)]",
        highlight === "active" &&
          "border-[var(--color-accent-ring)] bg-[var(--color-accent-soft)]",
        highlight === "warning" && "border-warning/40",
        highlight === "danger" && "border-danger/40",
      )}
    >
      <div className="flex items-center gap-1.5">
        <p className="text-[11px] font-medium uppercase tracking-wider text-text-dim">
          {label}
        </p>
        {live && (
          <span className="iris-pulse h-1.5 w-1.5 rounded-full bg-accent" />
        )}
      </div>
      <p
        className={cn(
          "mt-1 text-xl font-semibold tabular-nums leading-tight",
          mono && "font-mono text-lg",
          highlight === "warning" && "text-warning",
          highlight === "danger" && "text-danger",
          highlight === "active" && "text-accent",
        )}
      >
        {value}
      </p>
    </div>
  );
}

function formatUptime(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}
