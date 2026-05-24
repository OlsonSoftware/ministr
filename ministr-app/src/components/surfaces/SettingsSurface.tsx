import { useState } from "react";
import { Settings, Bot, Info } from "lucide-react";
import { cn } from "../../lib/utils";
import type { DaemonStatus } from "../../lib/types";
import { GeneralSettings } from "./GeneralSettings";
import { AiAssistantsPanel } from "./AiAssistantsPanel";
import { AboutPanel } from "./AboutPanel";
import { AdaptiveSurface } from "../ui/adaptive-surface";
import { H2 } from "../ui/heading";

interface Props {
  status: DaemonStatus;
  activeCorpusId: string | null;
  theme: "system" | "dark" | "light";
  onThemeChange: (t: "system" | "dark" | "light") => void;
  onShowOnboarding: () => void;
  onRefresh: () => void;
  onOpenLogs: () => void;
}

const NAV_ITEMS = [
  { id: "general", label: "General", icon: Settings },
  { id: "ai", label: "AI assistants", icon: Bot },
  { id: "about", label: "About", icon: Info },
] as const;

type SectionId = (typeof NAV_ITEMS)[number]["id"];

export function SettingsSurface(props: Props) {
  const [active, setActive] = useState<SectionId>("general");

  return (
    <AdaptiveSurface>
      <div className="h-full flex flex-col @min-[900px]/surface:flex-row min-h-0">
        {/* Sidebar nav */}
        <nav className="hidden @min-[900px]/surface:flex flex-col gap-1 w-[200px] shrink-0 border-r border-border-soft p-4 pt-6">
          {NAV_ITEMS.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              type="button"
              onClick={() => setActive(id)}
              className={cn(
                "flex items-center gap-2.5 px-3 py-2 rounded-md text-sm font-medium text-left transition-colors duration-150",
                active === id
                  ? "bg-surface-overlay text-text"
                  : "text-text-muted hover:text-text hover:bg-surface-overlay/50",
              )}
            >
              <Icon className="h-4 w-4 shrink-0" strokeWidth={1.8} />
              {label}
            </button>
          ))}
        </nav>

        {/* Active view */}
        <div className="flex-1 min-h-0 overflow-y-auto p-5">
          {active === "general" && (
            <div>
              <H2>General</H2>
              <div className="mt-4">
                <GeneralSettings
                  status={props.status}
                  theme={props.theme}
                  onThemeChange={props.onThemeChange}
                  onRefresh={props.onRefresh}
                />
              </div>
            </div>
          )}

          {active === "ai" && (
            <div>
              <H2>AI assistants</H2>
              <div className="mt-4">
                <AiAssistantsPanel
                  corpora={props.status.corpora}
                  activeCorpusId={props.activeCorpusId}
                />
              </div>
            </div>
          )}

          {active === "about" && (
            <div>
              <H2>About</H2>
              <div className="mt-4">
                <AboutPanel
                  status={props.status}
                  onShowOnboarding={props.onShowOnboarding}
                  onRefresh={props.onRefresh}
                  onOpenLogs={props.onOpenLogs}
                />
              </div>
            </div>
          )}
        </div>
      </div>
    </AdaptiveSurface>
  );
}
