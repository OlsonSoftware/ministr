import { useState } from "react";
import { cn } from "../../lib/utils";
import type { DaemonStatus } from "../../lib/types";
import { GeneralSettings } from "./GeneralSettings";
import { AiAssistantsPanel } from "./AiAssistantsPanel";
import { ServerSettings } from "./ServerSettings";
import { DeveloperPanel } from "./DeveloperPanel";
import { AboutPanel } from "./AboutPanel";

type SettingsTab = "general" | "ai" | "server" | "developer" | "about";

interface Tab {
  id: SettingsTab;
  label: string;
}

const TABS: Tab[] = [
  { id: "general", label: "General" },
  { id: "ai", label: "AI assistants" },
  { id: "server", label: "Server" },
  { id: "developer", label: "Developer" },
  { id: "about", label: "About" },
];

interface Props {
  status: DaemonStatus;
  activeCorpusId: string | null;
  setActiveCorpusId: (id: string | null) => void;
  theme: "system" | "dark" | "light";
  onThemeChange: (t: "system" | "dark" | "light") => void;
  onShowOnboarding: () => void;
  onRefresh: () => void;
  onOpenLogs: () => void;
}

/**
 * Settings surface — top-level destination, organised into five tabs:
 *
 * - General: theme, density, default surface, autostart.
 * - AI assistants: the MCP wizard — one row per detected client with
 *   one-click connect + live verification.
 * - Server: read-only ministr-server vitals + collapsible diagnostics
 *   (log viewer, context simulator).
 * - Developer: Sessions / Logs / Explore / Query playground that used
 *   to live as top-level tabs and drawers.
 * - About: version, maintenance, and the type-to-confirm danger zone.
 */
export function SettingsSurface(props: Props) {
  const [tab, setTab] = useState<SettingsTab>("general");

  return (
    <div className="h-full flex flex-col min-h-0">
      <nav
        aria-label="Settings sections"
        className="flex items-stretch gap-0 border-b-2 border-border bg-surface px-3 shrink-0"
      >
        {TABS.map((t) => {
          const active = t.id === tab;
          return (
            <button
              key={t.id}
              type="button"
              onClick={() => setTab(t.id)}
              aria-current={active ? "page" : undefined}
              className={cn(
                "px-4 py-2.5 cursor-pointer transition-none -mb-[2px]",
                "border-b-[3px]",
                "font-mono text-xs font-semibold uppercase tracking-[0.05em]",
                active
                  ? "border-b-accent text-text"
                  : "border-b-transparent text-text-muted hover:text-text",
              )}
            >
              {t.label}
            </button>
          );
        })}
      </nav>

      <div className="flex-1 min-h-0 overflow-y-auto p-5">
        {tab === "general" && (
          <GeneralSettings
            status={props.status}
            theme={props.theme}
            onThemeChange={props.onThemeChange}
            onRefresh={props.onRefresh}
          />
        )}
        {tab === "ai" && (
          <AiAssistantsPanel
            corpora={props.status.corpora}
            activeCorpusId={props.activeCorpusId}
          />
        )}
        {tab === "server" && <ServerSettings status={props.status} />}
        {tab === "developer" && (
          <DeveloperPanel
            status={props.status}
            activeCorpusId={props.activeCorpusId}
            setActiveCorpusId={props.setActiveCorpusId}
          />
        )}
        {tab === "about" && (
          <AboutPanel
            status={props.status}
            onShowOnboarding={props.onShowOnboarding}
            onRefresh={props.onRefresh}
            onOpenLogs={props.onOpenLogs}
          />
        )}
      </div>
    </div>
  );
}
