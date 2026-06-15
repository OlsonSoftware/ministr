import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { corpusFreshnessSummary, triggerReindex } from "../../lib/ipc";
import type { CorpusInfo } from "../../lib/ipc";
import { usePoll } from "../../lib/usePoll";
import { summarizeCounts } from "../../lib/trustSummary";
import { relTime } from "../../lib/relTime";
import { Screen } from "../ui/Screen";
import { ShellHeader } from "../ui/ShellHeader";
import { BackButton } from "../ui/BackButton";
import { ActionChip } from "../ui/ActionChip";
import { IndexingInstrument } from "../ui/IndexingInstrument";
import { ProjectCard } from "./ProjectCard";
import type { ProjectCardData } from "./ProjectCard";
import { ExpertConfig } from "../mirror/ExpertConfig";
import { RemoveProject } from "../ui/RemoveProject";
import { useIngestionProgress } from "../../lib/useIngestionProgress";

/**
 * ProjectManage (GUI v6, gui-v6-index-manage-panel) — the project-detail
 * view is now an index MANAGER, replacing the cut ProjectMirror file-tree.
 * A calm, data-forward panel of the controls the daemon already exposes:
 * a status summary (the v6 card), re-read/reindex with live progress, the
 * embedding-model config, the indexed paths, and remove. No file-tree of
 * checkmarks, no prose status.
 */
export function ProjectManage({
  corpus,
  onBack,
}: {
  corpus: CorpusInfo;
  onBack: () => void;
}) {
  const { data: fresh } = usePoll(() => corpusFreshnessSummary(corpus.id), 4_000);
  const { progress } = useIngestionProgress(1_000);
  const live = progress.get(corpus.id);
  const [pendingAt, setPendingAt] = useState<number | null>(null);

  // Optimism yields to real data (or a 15s safety net), as elsewhere.
  useEffect(() => {
    if (pendingAt === null || !fresh) return;
    if (fresh.indexing || Date.now() - pendingAt > 15_000) setPendingAt(null);
  }, [fresh, pendingAt]);

  const indexing = (fresh?.indexing ?? false) || pendingAt !== null;
  const summary = summarizeCounts(corpus.display_name, {
    stale: fresh?.stale ?? 0,
    new: fresh?.new ?? 0,
    indexing,
  });

  const card: ProjectCardData = {
    name: corpus.display_name,
    status: summary.state,
    files: corpus.files_indexed,
    sections: corpus.sections_count,
    behind: (fresh?.stale ?? 0) + (fresh?.new ?? 0),
    agents: corpus.active_sessions,
    stack: corpus.stack ?? [],
    symbols: corpus.symbols_count,
    indexedAgo: corpus.last_indexed ? relTime(corpus.last_indexed) : undefined,
    progress: live,
  };

  const reindex = () => {
    setPendingAt(Date.now());
    void triggerReindex(corpus.id).catch(() => {});
  };
  const indexingLive = indexing && live?.running;

  return (
    <Screen
      width="2xl"
      align="start"
      header={
        <ShellHeader
          leading={<BackButton onClick={onBack} label="All projects" />}
          title={corpus.display_name}
          subtitle="manage index"
        />
      }
    >
      {/* status summary — the same card vocabulary as Home, display-only.
          headingLevel=2 so the card name sits under the page h1 (valid order). */}
      <ProjectCard data={card} headingLevel={2} />

      <ManageSection label="Indexing">
        {indexingLive && live ? (
          <IndexingInstrument progress={live} />
        ) : (
          // The summary card above carries files/sections; here just the
          // last-read context + the action (no duplicate stats).
          <div className="flex items-center justify-between gap-4">
            <p className="text-sm tabular-nums text-dim">
              {corpus.last_indexed
                ? `last read ${relTime(corpus.last_indexed)}`
                : "not indexed yet"}
            </p>
            <ActionChip onClick={reindex}>Re-read project</ActionChip>
          </div>
        )}
      </ManageSection>

      <ManageSection label="Model">
        <ExpertConfig
          corpusId={corpus.id}
          model={corpus.model}
          onSaved={() => setPendingAt(Date.now())}
        />
      </ManageSection>

      <ManageSection label="Indexed paths">
        <ul className="space-y-1">
          {corpus.paths.map((p) => (
            <li key={p} className="truncate font-mono text-xs text-dim">
              {p}
            </li>
          ))}
        </ul>
      </ManageSection>

      <ManageSection label="Remove">
        <RemoveProject
          corpusId={corpus.id}
          displayName={corpus.display_name}
          onRemoved={onBack}
        />
      </ManageSection>
    </Screen>
  );
}

/** One labeled management card — quiet section label over a bordered body. */
function ManageSection({ label, children }: { label: string; children: ReactNode }) {
  return (
    <section className="space-y-2">
      <h2 className="px-1 text-xs font-medium uppercase tracking-wide text-dim">
        {label}
      </h2>
      <div className="rounded-lg border border-line bg-surface p-4">{children}</div>
    </section>
  );
}
