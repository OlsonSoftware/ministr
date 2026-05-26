import { useState } from "react";
import { Settings, Bot, Info } from "lucide-react";
import type { DaemonStatus } from "../../lib/types";
import { GeneralSettings } from "./GeneralSettings";
import { AiAssistantsPanel } from "./AiAssistantsPanel";
import { AboutPanel } from "./AboutPanel";
import { SurfaceSidebar, type SidebarItem } from "../ui/surface-sidebar";

interface Props {
  status: DaemonStatus;
  activeCorpusId: string | null;
  theme: "system" | "dark" | "light";
  onThemeChange: (t: "system" | "dark" | "light") => void;
  onShowOnboarding: () => void;
  onRefresh: () => void;
  onOpenLogs: () => void;
}

const NAV_ITEMS: readonly SidebarItem[] = [
  { id: "general", label: "General", icon: Settings },
  { id: "ai", label: "AI assistants", icon: Bot },
  { id: "about", label: "About", icon: Info },
];

export function SettingsSurface(props: Props) {
  const [active, setActive] = useState("general");

  return (
    <SurfaceSidebar
      title="Settings"
      items={NAV_ITEMS}
      active={active}
      onSelect={setActive}
    >
      {active === "general" && (
        <GeneralSettings
          status={props.status}
          theme={props.theme}
          onThemeChange={props.onThemeChange}
          onRefresh={props.onRefresh}
        />
      )}

      {active === "ai" && (
        <AiAssistantsPanel
          corpora={props.status.corpora}
          activeCorpusId={props.activeCorpusId}
        />
      )}

      {active === "about" && (
        <AboutPanel
          status={props.status}
          onShowOnboarding={props.onShowOnboarding}
          onRefresh={props.onRefresh}
          onOpenLogs={props.onOpenLogs}
        />
      )}
    </SurfaceSidebar>
  );
}
