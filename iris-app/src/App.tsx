import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  FolderKanban,
  Activity,
  Settings as SettingsIcon,
  ScrollText,
  Zap,
} from "lucide-react";
import { useDaemonStatus } from "./hooks/useDaemonStatus";
import { useTheme } from "./hooks/useTheme";
import { ProjectList } from "./components/ProjectList";
import { ProjectDetail } from "./components/ProjectDetail";
import { Settings } from "./components/Settings";
import { LogViewer } from "./components/LogViewer";
import { Onboarding } from "./components/Onboarding";
import { cn } from "./lib/utils";

type Tab = "projects" | "health" | "logs" | "settings";

export function App() {
  const { status, error, refresh } = useDaemonStatus();
  const { theme, setTheme } = useTheme();
  const [tab, setTab] = useState<Tab>("projects");
  const [showOnboarding, setShowOnboarding] = useState(false);

  useEffect(() => {
    invoke<boolean>("should_show_onboarding").then((should) => {
      setShowOnboarding(should);
    });
  }, []);
  const [selectedCorpusId, setSelectedCorpusId] = useState<string | null>(null);

  const selectedCorpus = status?.corpora.find((c) => c.id === selectedCorpusId);

  if (showOnboarding) {
    return <Onboarding onDismiss={() => setShowOnboarding(false)} />;
  }

  return (
    <div className="flex h-screen flex-col">
      {/* Header */}
      <header className="flex items-center justify-between border-b border-border px-4 py-2.5 shrink-0">
        <div className="flex items-center gap-2">
          <Zap className="h-4 w-4 text-accent" />
          <span className="font-semibold text-sm">iris</span>
          {status && (
            <span className="text-xs text-text-dim">
              v{status.version}
            </span>
          )}
        </div>
        {status && (
          <div className="flex gap-3 text-xs text-text-dim">
            <span>{status.model} ({status.model_dimension}d)</span>
            <span>{status.memory_mb.toFixed(0)} MB</span>
          </div>
        )}
      </header>

      {/* Error banner */}
      {error && (
        <div className="border-b border-danger/30 bg-danger/5 px-4 py-2 text-xs text-danger">
          {error}
        </div>
      )}

      {/* Main content */}
      <div className="flex flex-1 min-h-0">
        {/* Sidebar (wide screens) / bottom tabs (narrow) */}
        <nav className="hidden sm:flex flex-col w-10 border-r border-border py-2 items-center gap-1 shrink-0">
          <NavButton
            icon={FolderKanban}
            active={tab === "projects"}
            onClick={() => setTab("projects")}
            label="Projects"
          />
          <NavButton
            icon={Activity}
            active={tab === "health"}
            onClick={() => setTab("health")}
            label="Health"
          />
          <NavButton
            icon={ScrollText}
            active={tab === "logs"}
            onClick={() => setTab("logs")}
            label="Logs"
          />
          <div className="flex-1" />
          <NavButton
            icon={SettingsIcon}
            active={tab === "settings"}
            onClick={() => setTab("settings")}
            label="Settings"
          />
        </nav>

        {/* Content area */}
        <main className="flex-1 overflow-y-auto p-4">
          {!status ? (
            <div className="flex items-center justify-center h-full text-text-dim text-sm">
              Connecting to daemon...
            </div>
          ) : tab === "projects" ? (
            <div className="flex gap-4 h-full">
              <div className={cn("flex-1 min-w-0", selectedCorpus && "max-w-[55%]")}>
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
          ) : tab === "logs" ? (
            <LogViewer />
          ) : (
            <Settings
              status={status}
              theme={theme}
              onThemeChange={setTheme}
            />
          )}
        </main>
      </div>

      {/* Bottom tabs (narrow screens) */}
      <nav className="flex sm:hidden border-t border-border shrink-0">
        <TabButton
          icon={FolderKanban}
          label="Projects"
          active={tab === "projects"}
          onClick={() => setTab("projects")}
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

function NavButton({
  icon: Icon,
  active,
  onClick,
  label,
}: {
  icon: React.ComponentType<{ className?: string }>;
  active: boolean;
  onClick: () => void;
  label: string;
}) {
  return (
    <button
      onClick={onClick}
      title={label}
      className={cn(
        "p-2 rounded-md transition-colors cursor-pointer",
        active
          ? "bg-accent/10 text-accent"
          : "text-text-dim hover:text-text hover:bg-surface-overlay",
      )}
    >
      <Icon className="h-4 w-4" />
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
        "flex-1 flex flex-col items-center gap-0.5 py-2 text-xs transition-colors cursor-pointer",
        active ? "text-accent" : "text-text-dim",
      )}
    >
      <Icon className="h-4 w-4" />
      {label}
    </button>
  );
}

function HealthView({ status }: { status: import("./lib/types").DaemonStatus }) {
  const totalFiles = status.corpora.reduce((s, c) => s + c.files_indexed, 0);
  const totalSections = status.corpora.reduce((s, c) => s + c.sections_count, 0);
  const totalVectors = status.corpora.reduce((s, c) => s + c.embeddings_count, 0);
  const indexing = status.corpora.filter((c) => c.status.state === "indexing").length;
  const errors = status.corpora.filter((c) => c.status.state === "error").length;

  return (
    <div className="space-y-3">
      <h2 className="text-sm font-medium text-text-muted uppercase tracking-wider">
        Index Health
      </h2>
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <MetricCard label="Total Files" value={totalFiles.toLocaleString()} />
        <MetricCard label="Sections" value={totalSections.toLocaleString()} />
        <MetricCard label="Vectors" value={totalVectors.toLocaleString()} />
        <MetricCard label="Memory" value={`${status.memory_mb.toFixed(0)} MB`} />
      </div>
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <MetricCard label="Corpora" value={status.corpora.length.toString()} />
        <MetricCard label="Indexing" value={indexing.toString()} highlight={indexing > 0 ? "warning" : undefined} />
        <MetricCard label="Errors" value={errors.toString()} highlight={errors > 0 ? "danger" : undefined} />
        <MetricCard label="Uptime" value={formatUptime(status.uptime_secs)} />
      </div>
    </div>
  );
}

function MetricCard({
  label,
  value,
  highlight,
}: {
  label: string;
  value: string;
  highlight?: "warning" | "danger";
}) {
  return (
    <div className="rounded-lg border border-border bg-surface-raised p-3">
      <p className="text-xs text-text-dim">{label}</p>
      <p
        className={cn(
          "text-lg font-semibold mt-0.5",
          highlight === "warning" && "text-warning",
          highlight === "danger" && "text-danger",
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
