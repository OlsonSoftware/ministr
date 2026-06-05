/**
 * SystemSurface — the Account area's global "System" surface (AAA-VISION,
 * aaa-settings).
 *
 * Settings stops being a 5-tab sidebar of forms (general/ai/server/logs/about).
 * Per-project config already lives in the project's Tend facet; what's left is
 * genuinely GLOBAL, and it's presented as ONE thin, diagnostics-LED surface:
 *   1. Diagnostics — the daemon + index as a real health panel (not an About box)
 *   2. Integrations — AI assistants as LIVE connection cards (connected-now)
 *   3. Preferences — the few true globals (theme / density / autostart)
 *   4. Maintenance — data folder, logs, re-run onboarding, danger zone
 *
 * Built fresh on the v4 tokens + ui/ atoms (Card, Badge, MetricTile, StatusDot,
 * Toggle, Button, EmptyState) — NOT a re-skin of the retired SettingsSurface
 * sidebar. The pure `SystemSurface` renders from props so Storybook can drive
 * every state without Tauri; `SystemSurfaceConnector` wires the live hooks +
 * commands (useDensity, useMcpClients, set_autostart, open_path, …).
 */
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity,
  Boxes,
  Check,
  Cpu,
  ExternalLink,
  FolderOpen,
  Gauge,
  HardDrive,
  Loader2,
  MonitorSmartphone,
  Moon,
  Plug,
  Power,
  RefreshCw,
  Rocket,
  ScrollText,
  Sparkles,
  Sun,
  Terminal,
  Trash2,
} from "@/components/ui/icons";

import type { DaemonStatus } from "../../lib/types";
import type { Density } from "../../hooks/usePreferences";
import { useDensity, resetPreferences } from "../../hooks/usePreferences";
import {
  useMcpClients,
  type McpClientState,
  type McpClientView,
} from "../../hooks/useMcpClients";
import { corpusRoot } from "../../lib/corpus";
import { cn } from "../../lib/utils";

import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Card } from "../ui/card";
import { ConfirmDialog } from "../ui/confirm-dialog";
import { EmptyState } from "../ui/empty-state";
import { MetricTile } from "../ui/metric-tile";
import { StatusDot } from "../ui/status-dot";
import { Toggle } from "../ui/toggle";
import { useToast } from "../shell/ToastTray";

type ThemeChoice = "system" | "dark" | "light";

/** Mirror of the Rust `SetupStatus` (commands.rs::setup_status) — first-run
 *  setup state, relocated out of the retired onboarding wizard into the
 *  System diagnostics (aaa-onboarding-setup-mcp-relocate). */
export interface SetupStatus {
  cli_on_path: boolean;
  cli_path: string | null;
  data_dir: string;
  version: string;
}

export interface SystemSurfaceProps {
  status: DaemonStatus;
  theme: ThemeChoice;
  density: Density;
  /** First-run CLI-on-PATH setup state, or null while loading. */
  setup?: SetupStatus | null;
  /** True while the one-click PATH repair is running. */
  fixingPath?: boolean;
  onFixPath?: () => void;
  /** Live AI-assistant integration views (from useMcpClients). */
  integrations: McpClientView[];
  integrationsLoading?: boolean;
  /** Client id currently mid-action (connect/test), or null. */
  integrationsBusy?: string | null;
  /** The project root the integrations configure against (null = no project). */
  projectRoot?: string | null;
  onThemeChange: (t: ThemeChoice) => void;
  onDensityChange: (d: Density) => void;
  onToggleAutostart: () => void;
  onConnectIntegration: (id: string) => void;
  onTestIntegration: (id: string) => void;
  onOpenIntegrationFile: (path: string) => void;
  onRefreshIntegrations: () => void;
  onOpenDataFolder: () => void;
  onOpenLogs: () => void;
  onRerunOnboarding: () => void;
  onResetPreferences: () => void;
  onClearCache: () => void;
  onCheckUpdates: () => void;
}

function formatUptime(secs: number): string {
  if (secs < 60) return `${Math.max(0, Math.round(secs))}s`;
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ${m % 60}m`;
  const d = Math.floor(h / 24);
  return `${d}d ${h % 24}h`;
}

/** A labelled section — the repeated shell each System concern lives in. */
function Section({
  icon: Icon,
  title,
  meta,
  children,
}: {
  icon: typeof Gauge;
  title: string;
  meta?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="space-y-2.5">
      <div className="flex items-center gap-2">
        <Icon className="h-3.5 w-3.5 text-accent" strokeWidth={2.25} />
        <h3 className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
          {title}
        </h3>
        {meta && <div className="ml-auto">{meta}</div>}
      </div>
      {children}
    </section>
  );
}

export function SystemSurface(props: SystemSurfaceProps) {
  const { status, theme, density, integrations, setup } = props;

  // Aggregate index health across every registered corpus — the diagnostics
  // lens (is the whole fleet healthy?), distinct from per-project Tend care.
  const health = useMemo(() => {
    let files = 0;
    let vectors = 0;
    let indexing = 0;
    let errors = 0;
    for (const c of status.corpora) {
      files += c.files_indexed;
      vectors += c.embeddings_count;
      if (c.status.state === "indexing") indexing += 1;
      if (c.status.state === "error") errors += 1;
    }
    return { files, vectors, indexing, errors, projects: status.corpora.length };
  }, [status.corpora]);

  const fleetTone =
    health.errors > 0 ? "danger" : health.indexing > 0 ? "warning" : "success";
  const fleetLabel =
    health.errors > 0
      ? `${health.errors} error${health.errors > 1 ? "s" : ""}`
      : health.indexing > 0
        ? `${health.indexing} indexing`
        : "Healthy";

  return (
    <div className="h-full overflow-y-auto px-5 py-5 space-y-6">
      {/* ── Diagnostics — the system as a command-deck status hero. ──────────
          The first thing on the Account surface: a raised-tier banner with a
          lit top edge (brighter when the fleet is healthy) + a glowing system
          medallion, mirroring the ScopeHeader spine-object deck. Tone rides
          the medallion glow / status dot / health pill border; every readout
          value stays full-contrast text-* for AA. */}
      <section className="space-y-2.5">
        <div className="flex items-center gap-2">
          <Gauge className="h-3.5 w-3.5 text-accent" strokeWidth={2.25} />
          <h3 className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
            Diagnostics
          </h3>
        </div>

        <div className="relative overflow-hidden rounded-xl border border-border bg-surface-raised shadow-sm">
          <span
            aria-hidden
            className={cn(
              "pointer-events-none absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent to-transparent",
              fleetTone === "success" ? "via-accent/50" : "via-border-hover",
            )}
          />
          <div className="space-y-3.5 px-4 py-3.5">
            {/* Identity — glowing system medallion + daemon vitals + health pill. */}
            <div className="flex min-w-0 items-center gap-3">
              <span
                aria-hidden
                className={cn(
                  "relative grid h-11 w-11 shrink-0 place-items-center rounded-xl border bg-surface-overlay",
                  fleetTone === "success"
                    ? "border-accent/50 text-accent shadow-[var(--glow-soft)]"
                    : "border-border text-text-muted",
                )}
              >
                <Gauge className="h-[18px] w-[18px]" strokeWidth={2} />
                <span className="absolute -right-1 -top-1 grid place-items-center rounded-full bg-surface-raised p-0.5">
                  <StatusDot
                    tone={fleetTone}
                    pulse={fleetTone === "success" ? "live" : "off"}
                    size="md"
                  />
                </span>
              </span>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="truncate text-[15px] font-semibold text-text">
                    Daemon running
                  </span>
                  <Badge
                    variant={
                      fleetTone === "danger"
                        ? "danger"
                        : fleetTone === "warning"
                          ? "warning"
                          : "success"
                    }
                    dot
                  >
                    {fleetLabel}
                  </Badge>
                </div>
                <p className="mt-0.5 truncate font-mono text-mono-mini text-text-dim">
                  v{status.version} · up {formatUptime(status.uptime_secs)} ·{" "}
                  {status.total_sessions.toLocaleString()} session
                  {status.total_sessions === 1 ? "" : "s"}
                </p>
              </div>
            </div>

            {/* Vitals — the divided readout cluster. Kept at 2/3 cols: the
                Account panel is far narrower than the viewport, so a 6-up row
                (a viewport breakpoint can't see the panel width) would truncate
                the model + health values. Legibility wins over a single row. */}
            <div className="grid grid-cols-2 gap-px overflow-hidden rounded-lg border border-border bg-border sm:grid-cols-3">
              <MetricTile
                variant="cell"
                className="bg-surface"
                icon={Cpu}
                label="Model"
                value={`${status.model} · ${status.model_dimension}d`}
              />
              <MetricTile
                variant="cell"
                className="bg-surface"
                icon={HardDrive}
                label="Memory"
                value={`${status.memory_mb.toFixed(0)} MB`}
              />
              <MetricTile
                variant="cell"
                className="bg-surface"
                icon={Boxes}
                label="Projects"
                value={health.projects.toLocaleString()}
              />
              <MetricTile
                variant="cell"
                className="bg-surface"
                icon={ScrollText}
                label="Files"
                value={health.files.toLocaleString()}
              />
              <MetricTile
                variant="cell"
                className="bg-surface"
                icon={Activity}
                label="Vectors"
                value={health.vectors.toLocaleString()}
              />
              <MetricTile
                variant="cell"
                className="bg-surface"
                icon={Gauge}
                label="Index health"
                tone={fleetTone}
                value={fleetLabel}
              />
            </div>

            {/* CLI-on-PATH — relocated out of the retired onboarding wizard.
                A passive health row here, not a first-run gate. */}
            {setup && (
              <div className="flex items-center gap-3 rounded-md border border-border-soft bg-surface px-3 py-2.5">
                <StatusDot tone={setup.cli_on_path ? "success" : "danger"} />
                <div className="min-w-0 flex-1">
                  <p className="font-sans text-sm font-medium text-text">
                    ministr CLI on PATH
                  </p>
                  <p className="mt-0.5 truncate font-mono text-mono-mini text-text-dim">
                    {setup.cli_on_path
                      ? (setup.cli_path ?? "resolved")
                      : "not resolvable from this app — repair to let editors find it"}
                  </p>
                </div>
                {!setup.cli_on_path && props.onFixPath && (
                  <Button
                    size="sm"
                    onClick={props.onFixPath}
                    disabled={props.fixingPath}
                    className="shrink-0"
                  >
                    {props.fixingPath ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
                    ) : (
                      <Terminal className="h-3.5 w-3.5" strokeWidth={2} />
                    )}
                    Fix PATH
                  </Button>
                )}
              </div>
            )}
          </div>
        </div>
      </section>

      {/* ── Integrations — AI assistants as LIVE connection cards. ────────── */}
      <Section
        icon={Plug}
        title="AI integrations"
        meta={
          <Button
            variant="ghost"
            size="sm"
            onClick={props.onRefreshIntegrations}
            disabled={props.integrationsLoading}
          >
            {props.integrationsLoading ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" strokeWidth={2} />
            )}
            Refresh
          </Button>
        }
      >
        {!props.projectRoot ? (
          <EmptyState
            icon={Plug}
            title="Add a project to connect editors"
            hint="The MCP setup writes per-project config files. Pick a project from the spine, then connect Claude Code, Cursor, or Copilot here."
          />
        ) : (
          <ul className="space-y-2">
            {integrations.map((view) => (
              <li key={view.info.id}>
                <IntegrationCard
                  view={view}
                  busy={props.integrationsBusy === view.info.id}
                  onConnect={() => props.onConnectIntegration(view.info.id)}
                  onTest={() => props.onTestIntegration(view.info.id)}
                  onOpenFile={() => props.onOpenIntegrationFile(view.info.config_path)}
                />
              </li>
            ))}
          </ul>
        )}
      </Section>

      {/* ── Preferences — the few genuine globals. ────────────────────────── */}
      <Section icon={Sparkles} title="Preferences">
        <Card className="p-4 space-y-4">
          <PrefRow label="Theme" hint="Adapts to the OS by default.">
            <Segmented
              value={theme}
              onChange={(v) => props.onThemeChange(v as ThemeChoice)}
              options={[
                { value: "system", label: "System", icon: MonitorSmartphone },
                { value: "dark", label: "Dark", icon: Moon },
                { value: "light", label: "Light", icon: Sun },
              ]}
            />
          </PrefRow>
          <PrefRow label="Density" hint="Compact tightens padding across cards.">
            <Segmented
              value={density}
              onChange={(v) => props.onDensityChange(v as Density)}
              options={[
                { value: "comfortable", label: "Comfort" },
                { value: "compact", label: "Compact" },
              ]}
            />
          </PrefRow>
          <PrefRow
            label="Start at login"
            hint="Run ministr at login so assistants attach instantly."
          >
            <Toggle
              enabled={status.autostart_enabled ?? null}
              onToggle={props.onToggleAutostart}
              ariaLabel="Start at login"
            />
          </PrefRow>
        </Card>
      </Section>

      {/* ── Maintenance — utilities + the danger zone, demoted. ───────────── */}
      <Section icon={Power} title="Maintenance">
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="subtle" size="sm" onClick={props.onOpenDataFolder}>
            <FolderOpen className="h-3.5 w-3.5" strokeWidth={2} />
            Data folder
          </Button>
          <Button variant="subtle" size="sm" onClick={props.onOpenLogs}>
            <ScrollText className="h-3.5 w-3.5" strokeWidth={2} />
            Logs
          </Button>
          <Button variant="subtle" size="sm" onClick={props.onRerunOnboarding}>
            <Rocket className="h-3.5 w-3.5" strokeWidth={2} />
            Re-run onboarding
          </Button>
          <span className="mx-1 h-5 w-px bg-border" aria-hidden />
          <Button variant="danger" size="sm" onClick={props.onResetPreferences}>
            <RefreshCw className="h-3.5 w-3.5" strokeWidth={2} />
            Reset preferences
          </Button>
          <Button variant="danger" size="sm" onClick={props.onClearCache}>
            <Trash2 className="h-3.5 w-3.5" strokeWidth={2} />
            Clear cache
          </Button>
        </div>
      </Section>

      {/* ── Footer — version + update check. ──────────────────────────────── */}
      <footer className="flex items-center justify-between gap-3 border-t border-border-soft pt-4 font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
        <span>
          ministr · v{status.version} · up{" "}
          <span className="tabular-nums text-text-dim">
            {formatUptime(status.uptime_secs)}
          </span>
        </span>
        <button
          onClick={props.onCheckUpdates}
          className="inline-flex items-center gap-1 rounded-md border border-border bg-surface px-2 py-0.5 text-text hover:bg-surface-overlay cursor-pointer transition-colors duration-150"
        >
          <ExternalLink className="h-3 w-3" strokeWidth={2.5} />
          Check for updates
        </button>
      </footer>
    </div>
  );
}

// ── Integration card ───────────────────────────────────────────────────────

const STATE_META: Record<
  McpClientState,
  { tone: "success" | "warning" | "danger" | "muted"; label: string }
> = {
  connected: { tone: "success", label: "Connected" },
  configured: { tone: "warning", label: "Config written · verify" },
  not_configured: { tone: "muted", label: "Not configured" },
  not_installed: { tone: "muted", label: "Not installed" },
};

function IntegrationCard({
  view,
  busy,
  onConnect,
  onTest,
  onOpenFile,
}: {
  view: McpClientView;
  busy: boolean;
  onConnect: () => void;
  onTest: () => void;
  onOpenFile: () => void;
}) {
  const { info, state, lastTest, lastTestAt } = view;
  const meta = STATE_META[state];
  const installed = state !== "not_installed";

  return (
    <Card className={cn("p-3.5", !installed && "opacity-60")}>
      <div className="flex items-center gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            {state === "connected" ? (
              <span className="grid h-4 w-4 place-items-center rounded-sm bg-success text-white shrink-0">
                <Check className="h-3 w-3" strokeWidth={3} />
              </span>
            ) : (
              <StatusDot tone={meta.tone} />
            )}
            <span className="font-sans text-sm font-medium text-text truncate">
              {info.display_name}
            </span>
            <Badge variant={meta.tone === "muted" ? "muted" : meta.tone}>
              {meta.label}
            </Badge>
          </div>
          <p className="font-mono text-[10px] text-text-dim truncate mt-1">
            {info.config_path}
          </p>
          {lastTest && lastTestAt && (
            <p
              className={cn(
                "font-mono text-[10px] mt-0.5 truncate",
                lastTest.ok ? "text-success" : "text-text-muted",
              )}
            >
              {lastTest.message}
            </p>
          )}
        </div>

        <div className="flex items-center gap-1.5 shrink-0">
          {state === "not_installed" && (
            <span className="font-mono text-mono-mini uppercase tracking-[0.06em] text-text-dim">
              Not installed
            </span>
          )}
          {state === "not_configured" && (
            <Button size="sm" onClick={onConnect} disabled={busy}>
              {busy ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
              ) : (
                <Sparkles className="h-3.5 w-3.5" strokeWidth={2} />
              )}
              Connect
            </Button>
          )}
          {(state === "configured" || state === "connected") && (
            <>
              <Button variant="outline" size="sm" onClick={onTest} disabled={busy}>
                {busy ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
                ) : (
                  <RefreshCw className="h-3.5 w-3.5" strokeWidth={2} />
                )}
                Re-test
              </Button>
              <Button variant="ghost" size="sm" onClick={onOpenFile}>
                Open
              </Button>
            </>
          )}
        </div>
      </div>
    </Card>
  );
}

// ── Small layout helpers ─────────────────────────────────────────────────────

function PrefRow({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-4">
      <div className="min-w-0">
        <p className="font-sans text-sm font-medium text-text">{label}</p>
        {hint && <p className="font-sans text-xs text-text-dim mt-0.5">{hint}</p>}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

function Segmented({
  value,
  onChange,
  options,
}: {
  value: string;
  onChange: (v: string) => void;
  options: Array<{
    value: string;
    label: string;
    icon?: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  }>;
}) {
  return (
    <div className="flex items-center gap-0.5 rounded-md border border-border-soft bg-surface-overlay p-0.5">
      {options.map((o) => {
        const active = value === o.value;
        const Icon = o.icon;
        return (
          <button
            key={o.value}
            type="button"
            aria-pressed={active}
            onClick={() => onChange(o.value)}
            className={cn(
              "inline-flex items-center gap-1.5 h-7 px-2.5 rounded font-mono text-mono-mini uppercase tracking-[0.06em] cursor-pointer transition-colors duration-150",
              active
                ? "bg-surface text-text shadow-sm"
                : "text-text-dim hover:text-text",
            )}
          >
            {Icon && <Icon className="h-3 w-3" strokeWidth={2} />}
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

// ───────────────────────────────────────────────────────────────────────────
// Connector — wires the live hooks + Tauri commands. Drop-in for the System
// tab body (same props as the retired SettingsSurface).

const DATA_DIR = "~/.ministr/";
const RELEASES_URL = "https://ministr.ai";

interface ConnectorProps {
  status: DaemonStatus;
  activeCorpusId: string | null;
  theme: ThemeChoice;
  onThemeChange: (t: ThemeChoice) => void;
  onShowOnboarding: () => void;
  onRefresh: () => void;
  onOpenLogs: () => void;
}

export function SystemSurfaceConnector({
  status,
  activeCorpusId,
  theme,
  onThemeChange,
  onShowOnboarding,
  onRefresh,
  onOpenLogs,
}: ConnectorProps) {
  const { density, setDensity } = useDensity();
  const { toast } = useToast();

  const [setup, setSetup] = useState<SetupStatus | null>(null);
  const [fixingPath, setFixingPath] = useState(false);

  useEffect(() => {
    let cancelled = false;
    invoke<SetupStatus>("setup_status")
      .then((s) => !cancelled && setSetup(s))
      .catch(() => !cancelled && setSetup(null));
    return () => {
      cancelled = true;
    };
  }, []);

  async function fixPath() {
    setFixingPath(true);
    try {
      await invoke<string>("fix_path");
      const next = await invoke<SetupStatus>("setup_status");
      setSetup(next);
      toast(next.cli_on_path ? "PATH repaired" : "PATH still unresolved", {
        tone: next.cli_on_path ? "info" : "danger",
      });
    } catch (e) {
      toast("Fix PATH failed", { detail: String(e), tone: "danger" });
    } finally {
      setFixingPath(false);
    }
  }

  const corpus =
    status.corpora.find((c) => c.id === activeCorpusId) ??
    status.corpora[0] ??
    null;
  const projectRoot = corpus ? corpusRoot(corpus.paths) : null;
  const { views, loading, busy, connect, runTest, refresh } =
    useMcpClients(projectRoot);

  const [confirmReset, setConfirmReset] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);

  async function toggleAutostart() {
    const next = !(status.autostart_enabled ?? false);
    try {
      await invoke("set_autostart", { enabled: next });
      toast(next ? "Autostart enabled" : "Autostart disabled", { tone: "info" });
      onRefresh();
    } catch (e) {
      toast("Autostart change failed", { detail: String(e), tone: "danger" });
    }
  }

  function openPath(path: string, label: string) {
    invoke("open_path", { path })
      .then(() => toast(label, { tone: "info" }))
      .catch((e) => toast("Open failed", { detail: String(e), tone: "danger" }));
  }

  async function rerunOnboarding() {
    try {
      await invoke("reset_onboarding");
    } catch {
      /* non-fatal — still show the guide */
    }
    onShowOnboarding();
  }

  function performReset() {
    resetPreferences();
    setConfirmReset(false);
    toast("Preferences reset", { tone: "info" });
    setTimeout(() => window.location.reload(), 200);
  }

  async function performClearCache() {
    setConfirmClear(false);
    let cleared = 0;
    for (const c of status.corpora) {
      try {
        await invoke("trigger_reindex", { corpusId: c.id });
        cleared += 1;
      } catch {
        /* skip */
      }
    }
    toast("Cache cleared", {
      detail: `${cleared} ${cleared === 1 ? "project" : "projects"} re-indexing`,
      tone: "info",
    });
    onRefresh();
  }

  return (
    <>
      <SystemSurface
        status={status}
        theme={theme}
        density={density}
        setup={setup}
        fixingPath={fixingPath}
        onFixPath={fixPath}
        integrations={views}
        integrationsLoading={loading}
        integrationsBusy={busy}
        projectRoot={projectRoot}
        onThemeChange={onThemeChange}
        onDensityChange={setDensity}
        onToggleAutostart={toggleAutostart}
        onConnectIntegration={connect}
        onTestIntegration={runTest}
        onOpenIntegrationFile={(p) => openPath(p, "Config opened")}
        onRefreshIntegrations={refresh}
        onOpenDataFolder={() => openPath(DATA_DIR, "Data folder opened")}
        onOpenLogs={() => {
          if (status.log_path) openPath(status.log_path, "Log opened");
          else onOpenLogs();
        }}
        onRerunOnboarding={rerunOnboarding}
        onResetPreferences={() => setConfirmReset(true)}
        onClearCache={() => setConfirmClear(true)}
        onCheckUpdates={() => openPath(RELEASES_URL, "Opening releases")}
      />

      <ConfirmDialog
        open={confirmReset}
        title="Reset preferences"
        tone="danger"
        confirmLabel="Reset"
        confirmToken="RESET"
        body={
          <>
            <p>Clears local preferences (theme, density, drawer + active project).</p>
            <p className="font-sans text-xs tracking-[0.08em] text-text-dim mt-2">
              Your project list is not touched.
            </p>
          </>
        }
        onCancel={() => setConfirmReset(false)}
        onConfirm={performReset}
      />

      <ConfirmDialog
        open={confirmClear}
        title="Clear cache"
        tone="danger"
        confirmLabel="Clear cache"
        confirmToken="CLEAR CACHE"
        body={
          <>
            <p>
              Drops indexes for{" "}
              <span className="font-bold">
                all {status.corpora.length}{" "}
                {status.corpora.length === 1 ? "project" : "projects"}
              </span>{" "}
              and re-indexes each.
            </p>
            <p className="font-sans text-xs tracking-[0.08em] text-text-dim mt-2">
              Source files on disk are not touched.
            </p>
          </>
        }
        onCancel={() => setConfirmClear(false)}
        onConfirm={performClearCache}
      />
    </>
  );
}
