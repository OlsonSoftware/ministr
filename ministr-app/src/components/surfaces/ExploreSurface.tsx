import { useState } from "react";
import { Server, ScrollText, Search, FlaskConical } from "lucide-react";
import type { DaemonStatus } from "../../lib/types";
import { SurfaceSidebar, type SidebarItem } from "../ui/surface-sidebar";
import { ServerSettings } from "./ServerSettings";
import { LogViewer } from "../LogViewer";
import { ExploreView } from "../ExploreView";
import { QueryPlayground } from "../QueryPlayground";

interface Props {
  status: DaemonStatus;
  activeCorpusId: string | null;
  setActiveCorpusId: (id: string | null) => void;
}

const NAV_ITEMS: readonly SidebarItem[] = [
  { id: "server", label: "Server", icon: Server },
  { id: "logs", label: "Logs", icon: ScrollText },
  { id: "explorer", label: "Explorer", icon: Search },
  { id: "playground", label: "Playground", icon: FlaskConical },
];

export function ExploreSurface({
  status,
  activeCorpusId,
  setActiveCorpusId,
}: Props) {
  const [active, setActive] = useState("server");

  return (
    <SurfaceSidebar
      title="Explore"
      items={NAV_ITEMS}
      active={active}
      onSelect={setActive}
    >
      {active === "server" && <ServerSettings status={status} />}

      {active === "logs" && (
        <div className="h-[600px]">
          <LogViewer />
        </div>
      )}

      {active === "explorer" && (
        <ExploreView
          status={status}
          activeCorpusId={activeCorpusId}
          setActiveCorpusId={setActiveCorpusId}
        />
      )}

      {active === "playground" && (
        <QueryPlayground
          status={status}
          activeCorpusId={activeCorpusId}
          setActiveCorpusId={setActiveCorpusId}
        />
      )}
    </SurfaceSidebar>
  );
}
