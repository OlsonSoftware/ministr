import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Palette,
  HardDrive,
  Cpu,
  Power,
  ScrollText,
  Sun,
  Moon,
  MonitorSmartphone,
  Rocket,
} from "lucide-react";
import { Card } from "./ui/card";
import { Button } from "./ui/button";
import { Badge } from "./ui/badge";
import { cn } from "../lib/utils";
import type { DaemonStatus } from "../lib/types";

interface SettingsProps {
  status: DaemonStatus;
  theme: string;
  onThemeChange: (theme: "dark" | "light" | "system") => void;
  onShowOnboarding: () => void;
}

export function Settings({
  status,
  theme,
  onThemeChange,
  onShowOnboarding,
}: SettingsProps) {
  const [autostart, setAutostart] = useState<boolean | null>(null);

  useEffect(() => {
    invoke<boolean>("is_autostart_enabled").then(setAutostart).catch(() => {});
  }, []);

  async function toggleAutostart() {
    const next = !autostart;
    await invoke("set_autostart", { enabled: next });
    setAutostart(next);
  }

  const themeOptions = [
    { key: "system" as const, label: "System", icon: MonitorSmartphone },
    { key: "dark" as const, label: "Dark", icon: Moon },
    { key: "light" as const, label: "Light", icon: Sun },
  ];

  return (
    <div className="space-y-4 ministr-fade-in max-w-2xl">
      <header>
        <h2 className="text-base font-semibold text-text">Settings</h2>
        <p className="text-xs text-text-dim mt-0.5">
          Preferences for the ministr desktop app and daemon.
        </p>
      </header>

      <Section icon={Power} title="Startup" description="Launch the ministr tray app automatically at login.">
        <ToggleRow
          label="Start at login"
          description="Keeps the daemon running across reboots so MCP clients can attach instantly."
          enabled={autostart}
          onToggle={toggleAutostart}
        />
      </Section>

      <Section icon={Palette} title="Appearance" description="Pick a theme that matches your system.">
        <div className="flex gap-2">
          {themeOptions.map(({ key, label, icon: Icon }) => {
            const active = theme === key;
            return (
              <button
                key={key}
                onClick={() => onThemeChange(key)}
                className={cn(
                  "flex-1 inline-flex flex-col items-center gap-1.5 rounded-lg border px-3 py-3 text-xs font-medium cursor-pointer transition-all duration-120",
                  active
                    ? "border-[var(--color-accent-ring)] bg-[var(--color-accent-soft)] text-accent shadow-[0_0_0_3px_var(--color-accent-soft)]"
                    : "border-border/70 bg-surface-raised text-text-muted hover:border-border-hover hover:text-text",
                )}
              >
                <Icon className="h-4 w-4" />
                {label}
              </button>
            );
          })}
        </div>
      </Section>

      <Section
        icon={Cpu}
        title="Embedding model"
        description="Model used for semantic search across your corpora."
      >
        <Row label="Current" value={status.model} mono />
        <Row
          label="Dimension"
          value={<Badge variant="muted" className="font-mono">{status.model_dimension}d</Badge>}
        />
      </Section>

      <Section
        icon={HardDrive}
        title="Storage"
        description="Where ministr keeps its index and session data."
      >
        <Row label="Memory (RSS)" value={`${status.memory_mb.toFixed(0)} MB`} mono />
        <Row label="Data directory" value="~/.ministr/" mono />
      </Section>

      {status.log_path && (
        <Section
          icon={ScrollText}
          title="Log file"
          description="Where runtime logs are written."
        >
          <div className="font-mono text-[11px] text-text-muted bg-surface-sunken border border-border/60 rounded-md px-3 py-2 break-all select-all">
            {status.log_path}
          </div>
        </Section>
      )}

      <Section
        icon={Rocket}
        title="Onboarding"
        description="Replay the setup wizard to add or re-scan projects."
      >
        <div className="flex items-center justify-between gap-4">
          <p className="text-xs text-text-muted">
            Useful after adding <span className="font-mono">.ministr.toml</span>{" "}
            files to additional project roots.
          </p>
          <Button
            variant="outline"
            size="sm"
            onClick={async () => {
              await invoke("reset_onboarding");
              onShowOnboarding();
            }}
          >
            Show setup
          </Button>
        </div>
      </Section>
    </div>
  );
}

function Section({
  icon: Icon,
  title,
  description,
  children,
}: {
  icon: React.ComponentType<{ className?: string }>;
  title: string;
  description?: string;
  children: React.ReactNode;
}) {
  return (
    <Card hover="lift" className="p-5">
      <div className="flex items-start gap-3 mb-4">
        <div className="grid h-8 w-8 place-items-center rounded-lg bg-[var(--color-accent-soft)] text-accent shrink-0">
          <Icon className="h-4 w-4" />
        </div>
        <div className="flex-1">
          <h3 className="text-sm font-semibold text-text">{title}</h3>
          {description && (
            <p className="text-xs text-text-dim mt-0.5">{description}</p>
          )}
        </div>
      </div>
      <div className="space-y-2">{children}</div>
    </Card>
  );
}

function Row({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: React.ReactNode;
  mono?: boolean;
}) {
  return (
    <div className="flex items-center justify-between text-xs py-1 border-b border-border/40 last:border-0">
      <span className="text-text-muted">{label}</span>
      <span className={cn("text-text", mono && "font-mono tabular-nums")}>
        {value}
      </span>
    </div>
  );
}

function ToggleRow({
  label,
  description,
  enabled,
  onToggle,
}: {
  label: string;
  description?: string;
  enabled: boolean | null;
  onToggle: () => void;
}) {
  return (
    <div className="flex items-start justify-between gap-4">
      <div className="flex-1">
        <p className="text-sm font-medium text-text">{label}</p>
        {description && (
          <p className="text-xs text-text-dim mt-0.5">{description}</p>
        )}
      </div>
      <button
        onClick={onToggle}
        disabled={enabled === null}
        role="switch"
        aria-checked={!!enabled}
        className={cn(
          "relative h-6 w-10 shrink-0 rounded-full transition-colors duration-150 cursor-pointer",
          "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-accent-ring)]",
          enabled ? "bg-accent shadow-[inset_0_1px_0_rgb(255_255_255/0.2)]" : "bg-surface-overlay",
          enabled === null && "opacity-50 cursor-wait",
        )}
      >
        <span
          className={cn(
            "absolute top-0.5 left-0.5 h-5 w-5 rounded-full bg-white shadow-sm transition-transform duration-150",
            enabled && "translate-x-4",
          )}
        />
      </button>
    </div>
  );
}
