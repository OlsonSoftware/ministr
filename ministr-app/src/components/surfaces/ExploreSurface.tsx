import { useState } from "react";
import { Server, ScrollText, Code2 } from "lucide-react";
import type { DaemonStatus } from "../../lib/types";
import { SurfaceSidebar, type SidebarItem } from "../ui/surface-sidebar";
import { ServerSettings } from "./ServerSettings";
import { LogViewer } from "../LogViewer";
import { CodeBrowser } from "../code/CodeBrowser";

interface Props {
  status: DaemonStatus;
  activeCorpusId: string | null;
  setActiveCorpusId: (id: string | null) => void;
}

const NAV_ITEMS: readonly SidebarItem[] = [
  { id: "code", label: "Code", icon: Code2 },
  { id: "server", label: "Server", icon: Server },
  { id: "logs", label: "Logs", icon: ScrollText },
];

export function ExploreSurface({ status, activeCorpusId }: Props) {
  const [active, setActive] = useState("code");

  return (
    <SurfaceSidebar
      title="Explore"
      items={NAV_ITEMS}
      active={active}
      onSelect={setActive}
      fill={active === "code"}
    >
      {active === "code" && (
        <CodeBrowser status={status} activeCorpusId={activeCorpusId} />
      )}

      {active === "server" && <ServerSettings status={status} />}

      {active === "logs" && (
        <div className="h-[600px]">
          <LogViewer />
        </div>
      )}
    </SurfaceSidebar>
  );
}
