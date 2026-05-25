import { useState } from "react";
import { ScrollText, Search, FlaskConical } from "lucide-react";

import { cn } from "../../lib/utils";
import type { DaemonStatus } from "../../lib/types";
import { LogViewer } from "../LogViewer";
import { ExploreView } from "../ExploreView";
import { QueryPlayground } from "../QueryPlayground";
import { SettingsSection } from "./settings-primitives";
import { ContentTray } from "../ui/content-tray";

type DevTab = "logs" | "explore" | "playground";

const TABS: {
  id: DevTab;
  label: string;
  hint: string;
  icon: typeof ScrollText;
}[] = [
  { id: "logs", label: "Logs", hint: "Live daemon stderr stream.", icon: ScrollText },
  {
    id: "explore",
    label: "Explore",
    hint: "Raw section / symbol / bridge search.",
    icon: Search,
  },
  {
    id: "playground",
    label: "Playground",
    hint: "Hand-tune the retrieval pipeline.",
    icon: FlaskConical,
  },
];

interface Props {
  status: DaemonStatus;
  activeCorpusId: string | null;
  setActiveCorpusId: (id: string | null) => void;
}

export function DeveloperPanel({
  status,
  activeCorpusId,
  setActiveCorpusId,
}: Props) {
  const [tab, setTab] = useState<DevTab>("logs");
  const current = TABS.find((t) => t.id === tab) ?? TABS[0];

  return (
    <div className="space-y-3">
      <SettingsSection
        title="Developer tools"
        description="Power-user surfaces — logs, code explorer, query playground."
      />

      <div className="flex items-center gap-3">
        <nav
          aria-label="Developer sub-sections"
          className="flex gap-0"
        >
          {TABS.map((t) => {
            const active = t.id === tab;
            const Icon = t.icon;
            return (
              <button
                key={t.id}
                type="button"
                onClick={() => setTab(t.id)}
                aria-current={active ? "page" : undefined}
                className={cn(
                  "inline-flex items-center gap-1.5 border border-border-soft px-3 h-8 cursor-pointer transition-colors duration-150 ease-out -ml-[1px] first:ml-0 first:rounded-l-md last:rounded-r-md font-sans text-xs font-medium focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent focus-visible:z-20",
                  active
                    ? "border-accent bg-surface-overlay text-text z-10 relative"
                    : "bg-surface text-text-muted hover:text-text hover:bg-surface-overlay",
                )}
              >
                <Icon className="h-3.5 w-3.5" strokeWidth={2} />
                {t.label}
              </button>
            );
          })}
        </nav>
        <span className="font-sans text-xs text-text-dim">
          {current.hint}
        </span>
      </div>

      <ContentTray compact className="!p-0 overflow-hidden">
        {tab === "logs" && (
          <div className="h-[520px]">
            <LogViewer />
          </div>
        )}
        {tab === "explore" && (
          <div className="p-3">
            <ExploreView
              status={status}
              activeCorpusId={activeCorpusId}
              setActiveCorpusId={setActiveCorpusId}
            />
          </div>
        )}
        {tab === "playground" && (
          <div className="p-3">
            <QueryPlayground
              status={status}
              activeCorpusId={activeCorpusId}
              setActiveCorpusId={setActiveCorpusId}
            />
          </div>
        )}
      </ContentTray>
    </div>
  );
}
