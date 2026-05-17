/**
 * GeneralSettings — the everyday preferences panel.
 *
 * Theme, default surface on launch, density, autostart. Split out of the
 * old monolithic Settings.tsx; server info moved to ServerSettings and
 * maintenance/danger actions to AboutPanel.
 */
import { invoke } from "@tauri-apps/api/core";
import { MonitorSmartphone, Moon, Power, Sun } from "lucide-react";

import type { DaemonStatus } from "../../lib/types";
import { cn } from "../../lib/utils";
import {
  DEFAULT_TAB_OPTIONS,
  type DefaultTab,
  type Density,
  useDefaultTab,
  useDensity,
} from "../../hooks/usePreferences";
import { Toggle } from "../ui/toggle";
import { Zone } from "../ui/zone";
import { useToast } from "../shell/ToastTray";
import { PrefRow } from "./settings-primitives";

interface Props {
  status: DaemonStatus;
  theme: string;
  onThemeChange: (theme: "dark" | "light" | "system") => void;
  onRefresh: () => void;
}

export function GeneralSettings({
  status,
  theme,
  onThemeChange,
  onRefresh,
}: Props) {
  const autostart = status.autostart_enabled ?? null;
  const { defaultTab, setDefaultTab } = useDefaultTab();
  const { density, setDensity } = useDensity();
  const { toast } = useToast();

  async function toggleAutostart() {
    const next = !autostart;
    await invoke("set_autostart", { enabled: next });
    toast(next ? "AUTOSTART ENABLED" : "AUTOSTART DISABLED", { tone: "info" });
    onRefresh();
  }

  return (
    <div className="space-y-4 max-w-2xl mx-auto">
      <Zone title="PREFERENCES" tone="serif">
        <PrefRow label="THEME" description="Adapts to OS by default.">
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
                    "inline-flex flex-col items-center gap-1 border border-border-soft w-20 h-14 cursor-pointer transition-colors duration-150 ease-out -ml-[1px] first:ml-0 justify-center",
                    active
                      ? "border-accent bg-surface-overlay text-text z-10 relative"
                      : "bg-surface text-text-muted hover:bg-surface-overlay hover:text-text",
                  )}
                >
                  <Icon className="h-4 w-4" strokeWidth={2} />
                  <span className="font-sans text-xs font-medium">{label}</span>
                </button>
              );
            })}
          </div>
        </PrefRow>

        <PrefRow
          label="DEFAULT TAB"
          description="Which surface opens on launch."
        >
          <select
            value={defaultTab}
            onChange={(e) => setDefaultTab(e.target.value as DefaultTab)}
            className="h-9 border border-border-soft bg-surface px-2 text-sm font-sans font-medium text-text cursor-pointer focus:outline-none focus:border-accent transition-colors duration-150 ease-out rounded-md"
          >
            {DEFAULT_TAB_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </PrefRow>

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
                    "border border-border-soft px-3 h-9 cursor-pointer transition-colors duration-150 ease-out -ml-[1px] first:ml-0 font-sans text-sm font-medium",
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
    </div>
  );
}
