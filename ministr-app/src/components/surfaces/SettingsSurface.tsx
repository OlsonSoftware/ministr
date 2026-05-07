import { useState } from "react";
import { cn } from "../../lib/utils";
import type { DaemonStatus } from "../../lib/types";
import { Settings } from "../Settings";
import { AiAssistantsPanel } from "./AiAssistantsPanel";
import { DeveloperPanel } from "./DeveloperPanel";

type SettingsTab = "general" | "ai" | "developer";

interface Tab {
  id: SettingsTab;
  label: string;
}

const TABS: Tab[] = [
  { id: "general", label: "General" },
  { id: "ai", label: "AI assistants" },
  { id: "developer", label: "Developer" },
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
 * Settings surface — top-level destination, organised into tabs.
 *
 * - General: theme, density, default tab, autostart, daemon vitals.
 *   Wraps the existing Settings component as-is.
 * - AI assistants: the MCP wizard. One row per detected client with
 *   one-click connect + live verification (see AiAssistantsPanel).
 * - Developer: container for Sessions / Logs / Explore / Query
 *   playground that previously lived as top-level tabs and drawers.
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
          <Settings
            status={props.status}
            theme={props.theme}
            onThemeChange={props.onThemeChange}
            onShowOnboarding={props.onShowOnboarding}
            onRefresh={props.onRefresh}
            onOpenLogs={props.onOpenLogs}
          />
        )}
        {tab === "ai" && (
          <AiAssistantsPanel
            corpora={props.status.corpora}
            activeCorpusId={props.activeCorpusId}
          />
        )}
        {tab === "developer" && (
          <DeveloperPanel
            status={props.status}
            activeCorpusId={props.activeCorpusId}
            setActiveCorpusId={props.setActiveCorpusId}
          />
        )}
      </div>
    </div>
  );
}

