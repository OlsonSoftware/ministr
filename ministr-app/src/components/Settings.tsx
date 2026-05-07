import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  ExternalLink,
  FolderOpen,
  Moon,
  MonitorSmartphone,
  Power,
  RefreshCw,
  Rocket,
  ScrollText,
  Sun,
  Terminal,
  Trash2,
  X,
} from "lucide-react";
import { Button } from "./ui/button";
import { H1 } from "./ui/heading";
import { Toggle } from "./ui/toggle";
import { Zone } from "./ui/zone";
import { LogViewer } from "./LogViewer";
import { ContextSimulator } from "./ContextSimulator";
import { cn } from "../lib/utils";
import {
  DEFAULT_TAB_OPTIONS,
  type DefaultTab,
  type Density,
  resetPreferences,
  useDefaultTab,
  useDensity,
} from "../hooks/usePreferences";
import { useToast } from "./shell/ToastTray";
import type { DaemonStatus } from "../lib/types";

/** Detail payload for the `ministr-settings-scroll` window event. */
export type SettingsScrollTarget = "logs" | "simulator";

interface SettingsProps {
  status: DaemonStatus;
  theme: string;
  onThemeChange: (theme: "dark" | "light" | "system") => void;
  onShowOnboarding: () => void;
  onRefresh: () => void;
  /** Switch to a tab — used by `OPEN LOG FILE`. */
  onOpenLogs?: () => void;
}

const RELEASES_URL = "https://github.com/anthropics/ministr/releases";
const DATA_DIR = "~/.ministr/";

export function Settings({
  status,
  theme,
  onThemeChange,
  onShowOnboarding,
  onRefresh,
  onOpenLogs,
}: SettingsProps) {
  const autostart = status.autostart_enabled ?? null;
  const { defaultTab, setDefaultTab } = useDefaultTab();
  const { density, setDensity } = useDensity();
  const { toast } = useToast();
  const [confirmReset, setConfirmReset] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);
  const [logsExpanded, setLogsExpanded] = useState(false);
  const [simulatorExpanded, setSimulatorExpanded] = useState(false);
  const logsRef = useRef<HTMLDivElement>(null);
  const simulatorRef = useRef<HTMLDivElement>(null);

  // Listen for scroll-to-zone requests from the rest of the app
  // (DaemonDot's "open log file" fallback, palette nav:logs, etc.).
  // Phase 4 of the consolidation pass folded the standalone Logs and
  // Simulator tabs into this Diagnostics zone, so callers send us a
  // window event instead of switching to a route.
  useEffect(() => {
    function onScroll(e: Event) {
      const detail = (e as CustomEvent).detail as
        | SettingsScrollTarget
        | undefined;
      if (detail === "logs") {
        setLogsExpanded(true);
        requestAnimationFrame(() => {
          logsRef.current?.scrollIntoView({
            behavior: "smooth",
            block: "start",
          });
        });
      } else if (detail === "simulator") {
        setSimulatorExpanded(true);
        requestAnimationFrame(() => {
          simulatorRef.current?.scrollIntoView({
            behavior: "smooth",
            block: "start",
          });
        });
      }
    }
    window.addEventListener("ministr-settings-scroll", onScroll);
    return () => {
      window.removeEventListener("ministr-settings-scroll", onScroll);
    };
  }, []);

  async function toggleAutostart() {
    const next = !autostart;
    await invoke("set_autostart", { enabled: next });
    toast(next ? "AUTOSTART ENABLED" : "AUTOSTART DISABLED", { tone: "info" });
    onRefresh();
  }

  async function openDataFolder() {
    try {
      await invoke("open_path", { path: DATA_DIR });
      toast("DATA FOLDER OPENED", { tone: "info" });
    } catch (e) {
      toast("OPEN FAILED", { detail: String(e), tone: "danger" });
    }
  }

  async function openLogFile() {
    if (status.log_path) {
      try {
        await invoke("open_path", { path: status.log_path });
        toast("LOG FILE OPENED", { tone: "info" });
        return;
      } catch {
        /* fallback to in-app log viewer */
      }
    }
    onOpenLogs?.();
  }

  async function rerunOnboarding() {
    await invoke("reset_onboarding");
    onShowOnboarding();
  }

  function performResetPreferences() {
    resetPreferences();
    setConfirmReset(false);
    toast("PREFERENCES RESET", { tone: "info" });
    // Force-reload so theme/density and other preferences re-init from defaults.
    setTimeout(() => window.location.reload(), 200);
  }

  async function performClearCache() {
    setConfirmClear(false);
    let cleared = 0;
    for (const c of status.corpora) {
      try {
        await invoke("trigger_reindex", { corpusId: c.id });
        cleared++;
      } catch {
        /* skip */
      }
    }
    toast("CACHE CLEARED", {
      detail: `${cleared} ${cleared === 1 ? "project" : "projects"} re-indexing`,
      tone: "info",
    });
    onRefresh();
  }

  function checkForUpdates() {
    invoke("open_path", { path: RELEASES_URL }).catch(() => {});
  }

  function copyVersion() {
    navigator.clipboard
      .writeText(`ministr v${status.version}`)
      .then(() => toast("VERSION COPIED", { tone: "info" }))
      .catch(() => {});
  }

  return (
    <div className="space-y-4 max-w-2xl mx-auto">
      <header>
        <H1>Settings</H1>
        <p className="font-sans text-xs tracking-[0.05em] text-text-dim mt-1">
          Preferences · system · maintenance
        </p>
      </header>

      {/* PREFERENCES */}
      <Zone title="PREFERENCES" tone="serif">
        {/* Theme */}
        <PrefRow
          label="THEME"
          description="Adapts to OS by default."
        >
          <div className="flex gap-0">
            {(
              [
                { key: "system" as const, label: "SYSTEM", icon: MonitorSmartphone },
                { key: "dark" as const, label: "DARK", icon: Moon },
                { key: "light" as const, label: "LIGHT", icon: Sun },
              ]
            ).map(({ key, label, icon: Icon }) => {
              const active = theme === key;
              return (
                <button
                  key={key}
                  onClick={() => onThemeChange(key)}
                  className={cn(
                    "inline-flex flex-col items-center gap-1 border border-border-soft w-20 h-14 cursor-pointer transition-none -ml-[1px] first:ml-0 justify-center",
                    active
                      ? "border-accent bg-surface-overlay text-text z-10 relative"
                      : "bg-surface text-text-muted hover:bg-surface-overlay hover:text-text",
                  )}
                >
                  <Icon className="h-4 w-4" strokeWidth={2} />
                  <span className="font-sans text-xs font-medium">
                    {label}
                  </span>
                </button>
              );
            })}
          </div>
        </PrefRow>

        {/* Default tab */}
        <PrefRow label="DEFAULT TAB" description="Which tab opens on launch.">
          <select
            value={defaultTab}
            onChange={(e) => setDefaultTab(e.target.value as DefaultTab)}
            className="h-9 border border-border-soft bg-surface px-2 text-sm font-sans font-medium text-text cursor-pointer focus:outline-none focus:border-accent transition-none rounded-sm"
          >
            {DEFAULT_TAB_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </PrefRow>

        {/* Density */}
        <PrefRow
          label="DENSITY"
          description="Compact mode reduces padding across cards."
        >
          <div className="flex gap-0">
            {(
              [
                { key: "comfortable" as const, label: "COMFORT" },
                { key: "compact" as const, label: "COMPACT" },
              ]
            ).map(({ key, label }) => {
              const active = density === key;
              return (
                <button
                  key={key}
                  onClick={() => setDensity(key as Density)}
                  className={cn(
                    "border border-border-soft px-3 h-9 cursor-pointer transition-none -ml-[1px] first:ml-0 font-sans text-sm font-medium",
                    active
                      ? "border-accent bg-surface-overlay text-text z-10 relative"
                      : "bg-surface text-text-muted hover:text-text hover:bg-surface-overlay",
                  )}
                >
                  {label.charAt(0) + label.slice(1).toLowerCase()}
                </button>
              );
            })}
          </div>
        </PrefRow>

        {/* Autostart */}
        <PrefRow
          label="AUTOSTART"
          description="Run ministr at login so your AI assistants can attach instantly."
          icon={Power}
        >
          <Toggle
            enabled={autostart}
            onToggle={toggleAutostart}
            ariaLabel="Start at login"
          />
        </PrefRow>
      </Zone>

      {/* SERVER */}
      <Zone title="SERVER" subtitle="READ-ONLY" tone="serif">
        <MetaRow label="VERSION" value={`v${status.version}`} />
        <MetaRow label="EMBEDDING MODEL" value={status.model} />
        <MetaRow label="MEMORY" value={`${status.memory_mb.toFixed(0)} MB RSS`} />
        <MetaRow label="DATA DIR" value={DATA_DIR} />
        {status.log_path && (
          <MetaRow label="LOG FILE" value={status.log_path} truncate />
        )}
      </Zone>

      {/* MAINTENANCE */}
      <Zone title="MAINTENANCE" tone="serif">
        <div className="grid grid-cols-2 md:grid-cols-3 gap-0">
          <MaintAction
            icon={FolderOpen}
            label="OPEN DATA FOLDER"
            onClick={openDataFolder}
          />
          <MaintAction
            icon={ScrollText}
            label="OPEN LOG FILE"
            onClick={openLogFile}
          />
          <MaintAction
            icon={Rocket}
            label="RE-RUN ONBOARDING"
            onClick={rerunOnboarding}
          />
          <MaintAction
            icon={RefreshCw}
            label="RESET PREFERENCES"
            danger
            onClick={() => setConfirmReset(true)}
          />
          <MaintAction
            icon={Trash2}
            label="CLEAR CACHE"
            danger
            onClick={() => setConfirmClear(true)}
          />
        </div>
      </Zone>

      {/* DIAGNOSTICS — folds the previous Logs + Simulator tabs into a
          collapsible zone here. Both default-collapsed so Settings stays
          fast on cold open; users can click a header to expand, or
          dispatch `ministr-settings-scroll` from elsewhere in the app.
          (The Developer tab also surfaces the log viewer; this stays
          here so the maintenance + log path are reachable from one
          place.) */}
      <Zone title="DIAGNOSTICS" tone="serif">
        <div ref={logsRef}>
          <DiagnosticSection
            icon={ScrollText}
            label="Server log"
            hint="Recent log lines from the running ministr server"
            expanded={logsExpanded}
            onToggle={() => setLogsExpanded((v) => !v)}
            isLast={false}
          >
            <div className="max-h-[420px] overflow-hidden">
              <LogViewer />
            </div>
          </DiagnosticSection>
        </div>
        <div ref={simulatorRef}>
          <DiagnosticSection
            icon={Terminal}
            label="Context simulator"
            hint="Replay a project query against the current session model"
            expanded={simulatorExpanded}
            onToggle={() => setSimulatorExpanded((v) => !v)}
            isLast={true}
          >
            <ContextSimulator />
          </DiagnosticSection>
        </div>
      </Zone>

      {/* Version footer */}
      <footer className="flex items-center justify-between gap-3 border-t-2 border-border px-3 py-2 font-mono text-xs uppercase tracking-[0.05em] text-text-dim">
        <button
          onClick={copyVersion}
          title="Click to copy version"
          className="text-text-dim hover:text-text cursor-pointer"
        >
          MINISTR · v{status.version} · UPTIME{" "}
          <span className="tabular-nums text-text-dim">
            {formatUptime(status.uptime_secs)}
          </span>
        </button>
        <button
          onClick={checkForUpdates}
          className="inline-flex items-center gap-1 border-2 border-border bg-surface px-2 py-0.5 text-text hover:bg-surface-overlay hover:text-text cursor-pointer transition-none"
        >
          <ExternalLink className="h-3 w-3" strokeWidth={2.5} />
          Check for updates
        </button>
      </footer>

      {confirmReset && (
        <TypedConfirmModal
          title="RESET PREFERENCES"
          token="RESET"
          tone="danger"
          body={
            <>
              <p className="font-mono text-xs text-text leading-relaxed">
                Clears local-storage preferences (theme, default tab, density,
                drawer state, active project).
              </p>
              <p className="font-sans text-xs tracking-[0.05em] text-text-dim mt-2">
                Your project list is not touched.
              </p>
            </>
          }
          onCancel={() => setConfirmReset(false)}
          onConfirm={performResetPreferences}
        />
      )}
      {confirmClear && (
        <TypedConfirmModal
          title="CLEAR CACHE"
          token="CLEAR CACHE"
          tone="danger"
          body={
            <>
              <p className="font-mono text-xs text-text leading-relaxed">
                Drops indexes for{" "}
                <span className="font-bold uppercase">
                  ALL {status.corpora.length}{" "}
                  {status.corpora.length === 1 ? "PROJECT" : "PROJECTS"}
                </span>{" "}
                and triggers re-index of each.
              </p>
              <p className="font-sans text-xs tracking-[0.05em] text-text-dim mt-2">
                Source files on disk are not touched.
              </p>
            </>
          }
          onCancel={() => setConfirmClear(false)}
          onConfirm={performClearCache}
        />
      )}
    </div>
  );
}

// ─── ROW PRIMITIVES ────────────────────────────────────────────────────────

function DiagnosticSection({
  icon: Icon,
  label,
  hint,
  expanded,
  onToggle,
  isLast,
  children,
}: {
  icon: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  label: string;
  hint: string;
  expanded: boolean;
  onToggle: () => void;
  isLast: boolean;
  children: React.ReactNode;
}) {
  return (
    <>
      <button
        onClick={onToggle}
        className={cn(
          "flex w-full items-center gap-2 px-3 py-2 cursor-pointer hover:bg-surface-overlay transition-none text-left",
          !isLast || expanded ? "border-b border-border-soft" : "",
        )}
      >
        {expanded ? (
          <ChevronDown className="h-3.5 w-3.5 text-text-dim shrink-0" strokeWidth={2.5} />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 text-text-dim shrink-0" strokeWidth={2.5} />
        )}
        <Icon className="h-3.5 w-3.5 text-text-dim shrink-0" strokeWidth={2} />
        <span className="font-sans text-sm font-semibold text-text">
          {label}
        </span>
        <span className="font-sans text-xs text-text-dim truncate">
          · {hint}
        </span>
      </button>
      {expanded && (
        <div
          className={cn(
            "px-3 py-3",
            !isLast && "border-b border-border-soft",
          )}
        >
          {children}
        </div>
      )}
    </>
  );
}

function PrefRow({
  label,
  description,
  icon: Icon,
  children,
}: {
  label: string;
  description?: string;
  icon?: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  children: React.ReactNode;
}) {
  const sentence = /^[A-Z][A-Z\s\-—·]+$/.test(label)
    ? label.charAt(0) + label.slice(1).toLowerCase()
    : label;
  return (
    <div className="flex items-center justify-between gap-4 border-b border-border-soft last:border-b-0 px-3 py-3">
      <div className="min-w-0 flex-1 flex items-start gap-2">
        {Icon && <Icon className="h-3.5 w-3.5 text-text-dim mt-0.5 shrink-0" strokeWidth={2} />}
        <div className="min-w-0">
          <p className="font-sans text-sm font-semibold text-text">
            {sentence}
          </p>
          {description && (
            <p className="font-sans text-xs text-text-dim mt-0.5">
              {description}
            </p>
          )}
        </div>
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

function MetaRow({
  label,
  value,
  truncate,
}: {
  label: string;
  value: string;
  truncate?: boolean;
}) {
  return (
    <div className="flex items-center justify-between gap-3 border-b border-border-soft last:border-b-0 px-3 py-1.5">
      <span className="font-mono text-mono-mini uppercase tracking-[0.05em] text-text-dim shrink-0">
        {label}
      </span>
      <span
        className={cn(
          "font-mono text-xs tabular-nums text-text text-right",
          truncate && "truncate",
        )}
        title={value}
      >
        {value}
      </span>
    </div>
  );
}

function MaintAction({
  icon: Icon,
  label,
  danger,
  onClick,
}: {
  icon: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  label: string;
  danger?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "border border-border-soft px-3 py-3 flex flex-col items-center gap-2 cursor-pointer transition-none -ml-[1px] -mt-[1px] first:ml-0 first:mt-0",
        "bg-surface text-text-muted",
        danger
          ? "hover:bg-danger hover:text-white hover:border-danger"
          : "hover:bg-surface-overlay hover:text-text hover:border-border",
      )}
    >
      <Icon className="h-4 w-4" strokeWidth={2} />
      <span className="font-sans text-xs font-medium text-center">
        {label}
      </span>
    </button>
  );
}

// ─── TYPED-CONFIRM MODAL ───────────────────────────────────────────────────

function TypedConfirmModal({
  title,
  token,
  tone,
  body,
  onCancel,
  onConfirm,
}: {
  title: string;
  token: string;
  tone?: "danger";
  body: React.ReactNode;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const [typed, setTyped] = useState("");
  const match = typed.trim().toUpperCase() === token.toUpperCase();
  return (
    <div
      className="fixed inset-0 z-[1100] flex items-start justify-center bg-black/40 px-6"
      style={{ paddingTop: "20vh" }}
      role="dialog"
      aria-modal="true"
      onClick={onCancel}
    >
      <div
        className={cn(
          "w-full max-w-md border-2 bg-surface shadow-lg",
          tone === "danger" ? "border-danger" : "border-border",
        )}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          className={cn(
            "flex items-center justify-between border-b-2 bg-surface-overlay px-3 py-2",
            tone === "danger" ? "border-danger" : "border-border",
          )}
        >
          <span className="inline-flex items-center gap-2 font-mono text-mono-mini font-bold uppercase tracking-[0.05em] text-danger">
            <AlertTriangle className="h-3 w-3" strokeWidth={2.5} />
            {title}
          </span>
          <button
            onClick={onCancel}
            aria-label="Close"
            className="grid h-6 w-6 place-items-center border-2 border-border hover:bg-surface-overlay hover:text-text cursor-pointer transition-none"
          >
            <X className="h-3 w-3" strokeWidth={2.5} />
          </button>
        </div>
        <div className="p-4">
          {body}
          <div className="mt-4">
            <label className="font-mono text-xs tracking-[0.05em] text-text-dim block mb-1">
              TYPE <span className="text-text font-bold">{token}</span> To confirm
            </label>
            <input
              autoFocus
              value={typed}
              onChange={(e) => setTyped(e.target.value)}
              placeholder={token}
              className="h-9 w-full border border-border-soft bg-surface px-2 text-xs font-mono uppercase text-text placeholder:text-text-dim focus:outline-none focus:bg-surface-overlay transition-none"
            />
          </div>
          <div className="flex items-center gap-2 mt-4 justify-end">
            <Button variant="outline" size="sm" onClick={onCancel}>
              CANCEL
            </Button>
            <Button
              variant="danger"
              size="sm"
              onClick={onConfirm}
              disabled={!match}
            >
              {title}
            </Button>
          </div>
        </div>
      </div>
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
