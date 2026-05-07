import { useState } from "react";
import { cn } from "../../lib/utils";
import type { DaemonStatus } from "../../lib/types";
import { Settings } from "../Settings";
import { AiAssistantsPanel } from "./AiAssistantsPanel";

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
 *   M1 wraps the existing Settings component as-is.
 * - AI assistants: the MCP wizard. M1 shows a stub; M3 ships the real one.
 * - Developer: container for Sessions / Logs / Activity / Bridges /
 *   Query playground that previously lived as top-level tabs and drawers.
 *   M1 shows a placeholder explaining the migration; M4 ports the real
 *   panels in.
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
        {tab === "general" && <Settings {...props} />}
        {tab === "ai" && <AiAssistantsPanel />}
        {tab === "developer" && <DeveloperPlaceholder />}
      </div>
    </div>
  );
}

function DeveloperPlaceholder() {
  return (
    <div className="space-y-4">
      <header className="space-y-1">
        <h2 className="font-mono text-sm font-bold uppercase tracking-[0.05em] text-text">
          Developer tools
        </h2>
        <p className="font-serif text-sm text-text-muted">
          Power-user surfaces that used to be top-level tabs. Hidden here so
          the main UI stays focused on what most users need day to day.
        </p>
      </header>

      <ul className="grid grid-cols-1 md:grid-cols-2 gap-2">
        {[
          ["Sessions", "Per-agent budgets, dedup hits, compression ratio."],
          ["Logs", "Live daemon stderr stream."],
          ["Activity", "Recent ministr_* tool calls + file-change events."],
          ["Bridges", "Cross-language IPC link explorer."],
          ["Query playground", "Raw section / symbol / bridge search."],
        ].map(([title, desc]) => (
          <li
            key={title}
            className="border-2 border-border-soft bg-surface p-3"
          >
            <div className="font-mono text-xs font-semibold uppercase tracking-[0.05em] text-text">
              {title}
            </div>
            <p className="font-serif text-mono-sm text-text-muted mt-1">{desc}</p>
            <p className="font-mono text-mono-mini uppercase tracking-[0.05em] text-text-dim mt-2">
              Coming in M4
            </p>
          </li>
        ))}
      </ul>
    </div>
  );
}
