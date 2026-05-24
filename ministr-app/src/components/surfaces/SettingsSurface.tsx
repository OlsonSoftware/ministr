import { useState } from "react";
import { cn } from "../../lib/utils";
import type { DaemonStatus } from "../../lib/types";
import { GeneralSettings } from "./GeneralSettings";
import { AiAssistantsPanel } from "./AiAssistantsPanel";
import { LinkedProjectsPanel } from "./LinkedProjectsPanel";
import { AboutPanel } from "./AboutPanel";

type SettingsTab =
  | "general"
  | "ai"
  | "linked"
  | "about";

interface Tab {
  id: SettingsTab;
  label: string;
}

const TABS: Tab[] = [
  { id: "general", label: "General" },
  { id: "ai", label: "AI assistants" },
  { id: "linked", label: "Linked projects" },
  { id: "about", label: "About" },
];

interface Props {
  status: DaemonStatus;
  activeCorpusId: string | null;
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
                "px-4 py-2.5 cursor-pointer transition-colors duration-150 ease-out -mb-[2px]",
                "border-b-[3px]",
                "font-mono text-xs font-semibold uppercase tracking-[0.08em]",
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
        {tab === "linked" && (
          <LinkedProjectsPanel
            corpora={props.status.corpora}
            activeCorpusId={props.activeCorpusId}
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
