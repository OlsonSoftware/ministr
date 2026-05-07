/**
 * DeveloperPanel — power-user surfaces moved here from top-level tabs.
 *
 * What used to live on the main rail (Sessions, Explore, Query playground)
 * or as Workspace drawers (Logs) now hides behind Settings → Developer.
 * Each row reuses the existing component as-is so behavior parity is
 * preserved; the only thing that changed is the route by which the user
 * reaches them.
 *
 * The plan calls for these to be split into individual files under
 * `surfaces/settings/DeveloperTools/`. We keep them here as one file for
 * now — every sub-view is a one-line wrapper around an existing
 * component, so splitting would just add boilerplate without changing
 * behavior. A later cleanup pass can extract them when surface-specific
 * logic accumulates.
 */
import { useState } from "react";

import { cn } from "../../lib/utils";
import type { DaemonStatus } from "../../lib/types";
import { SessionDashboard } from "../SessionDashboard";
import { LogViewer } from "../LogViewer";
import { ExploreView } from "../ExploreView";
import { QueryPlayground } from "../QueryPlayground";

type DevTab = "sessions" | "logs" | "explore" | "playground";

const TABS: { id: DevTab; label: string; hint: string }[] = [
  {
    id: "sessions",
    label: "Sessions",
    hint: "Per-agent budgets, dedup hits, compression ratio.",
  },
  { id: "logs", label: "Logs", hint: "Live daemon stderr stream." },
  {
    id: "explore",
    label: "Explore",
    hint: "Raw section / symbol / bridge search.",
  },
  {
    id: "playground",
    label: "Query playground",
    hint: "Hand-tune the retrieval pipeline.",
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
  const [tab, setTab] = useState<DevTab>("sessions");
  const current = TABS.find((t) => t.id === tab) ?? TABS[0];

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

      <nav
        aria-label="Developer sub-sections"
        className="flex items-stretch gap-0 border-b-2 border-border bg-surface px-2"
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
                "px-3 py-2 cursor-pointer transition-none -mb-[2px]",
                "border-b-[3px]",
                "font-mono text-mono-mini font-semibold uppercase tracking-[0.05em]",
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

      <p className="font-serif italic text-mono-mini text-text-dim">
        {current.hint}
      </p>

      <div className="border-t border-border-soft pt-4">
        {tab === "sessions" && <SessionDashboard status={status} />}
        {tab === "logs" && (
          <div className="h-[600px]">
            <LogViewer />
          </div>
        )}
        {tab === "explore" && (
          <ExploreView
            status={status}
            activeCorpusId={activeCorpusId}
            setActiveCorpusId={setActiveCorpusId}
          />
        )}
        {tab === "playground" && (
          <QueryPlayground
            status={status}
            activeCorpusId={activeCorpusId}
            setActiveCorpusId={setActiveCorpusId}
          />
        )}
      </div>
    </div>
  );
}
