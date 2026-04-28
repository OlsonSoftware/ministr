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
import { Button } from "./ui/button";
import { Badge } from "./ui/badge";
import { LabeledCard } from "./ui/labeled-card";
import { LabeledRow } from "./ui/labeled-row";
import { ToggleRow } from "./ui/toggle";
import { cn } from "../lib/utils";
import { accentTone } from "../lib/ui-tokens";
import type { DaemonStatus } from "../lib/types";

interface SettingsProps {
  status: DaemonStatus;
  theme: string;
  onThemeChange: (theme: "dark" | "light" | "system") => void;
  onShowOnboarding: () => void;
  onRefresh: () => void;
}

export function Settings({
  status,
  theme,
  onThemeChange,
  onShowOnboarding,
  onRefresh,
}: SettingsProps) {
  // Autostart now rides on the daemon_status poll (Tauri side populates
  // it via `autolaunch().is_enabled()`). `undefined` while the first
  // status response is in flight; treated as a disabled-pending state.
  const autostart = status.autostart_enabled ?? null;

  async function toggleAutostart() {
    const next = !autostart;
    await invoke("set_autostart", { enabled: next });
    onRefresh();
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
      </header>

      <LabeledCard iconTone="accent" icon={Power} title="Startup">
        <ToggleRow
          label="Start at login"
          description="Keeps the daemon running across reboots so MCP clients can attach instantly."
          enabled={autostart}
          onToggle={toggleAutostart}
        />
      </LabeledCard>

      <LabeledCard iconTone="accent" icon={Palette} title="Appearance">
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
                    ? cn("border-[var(--color-accent-ring)]", accentTone)
                    : "border-border/70 bg-surface-raised text-text-muted hover:border-border-hover hover:text-text",
                )}
              >
                <Icon className="h-4 w-4" />
                {label}
              </button>
            );
          })}
        </div>
      </LabeledCard>

      <LabeledCard iconTone="accent" icon={Cpu} title="Embedding model">
        <LabeledRow bordered label="Current" value={status.model} mono />
        <LabeledRow
          bordered
          label="Dimension"
          value={<Badge variant="muted" className="font-mono">{status.model_dimension}d</Badge>}
        />
      </LabeledCard>

      <LabeledCard iconTone="accent" icon={HardDrive} title="Storage">
        <LabeledRow bordered label="Memory (RSS)" value={`${status.memory_mb.toFixed(0)} MB`} mono />
        <LabeledRow bordered label="Data directory" value="~/.ministr/" mono />
      </LabeledCard>

      {status.log_path && (
        <LabeledCard iconTone="accent" icon={ScrollText} title="Log file">
          <div className="font-mono text-[11px] text-text-muted bg-surface-sunken border border-border/60 rounded-md px-3 py-2 break-all select-all">
            {status.log_path}
          </div>
        </LabeledCard>
      )}

      <LabeledCard iconTone="accent" icon={Rocket} title="Onboarding">
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
      </LabeledCard>
    </div>
  );
}
