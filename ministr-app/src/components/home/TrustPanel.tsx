import { useEffect, useMemo, useState } from "react";
import { corpusFreshnessSummary, listCorpora, triggerReindex } from "../../lib/ipc";
import type { CorpusInfo, FreshnessSummary } from "../../lib/ipc";
import { usePoll } from "../../lib/usePoll";
import { summarizeCounts } from "../../lib/trustSummary";
import { relTime } from "../../lib/relTime";
import { StatusBanner } from "../ui/StatusBanner";
import { ActionChip } from "../ui/ActionChip";
import { Brand } from "../ui/Brand";
import { ShellHeader } from "../ui/ShellHeader";
import { SettingsMenu } from "../ui/SettingsMenu";
import { ConnectionNote } from "../ui/ConnectionNote";
import { Beat } from "../ui/Beat";
import { ProjectCard } from "../manager/ProjectCard";
import type { ProjectCardData } from "../manager/ProjectCard";
import { Screen } from "../ui/Screen";
import { useIngestionProgress } from "../../lib/useIngestionProgress";

/**
 * Home — the index MANAGER (GUI v6, gui-v6-visual-language). One visual
 * ProjectCard per index, worst-first: status as a colored rail + numeric
 * stat strip + icon actions, NOT a prose sentence. Healthy indexes stay
 * quiet; behind/indexing ones rise to the top.
 */
export function TrustPanel({
  onOpenProject,
  onAddProject,
}: {
  onOpenProject: (corpus: CorpusInfo) => void;
  // Required: the empty home must NEVER dead-end — the "Choose a folder…"
  // CTA is the only way out of a zero-project state, so every mount wires it.
  onAddProject: () => void;
}) {
  const { data: corpora, error } = usePoll(fetchAll, 5_000);
  // Live per-corpus indexing progress — drives the inline instrument on
  // any row that is updating (gui-indexing-instrument).
  const { progress } = useIngestionProgress(1_000);
  // Optimistic "catching up" per corpus, set when the daemon ACCEPTS a
  // reindex; real poll data (indexing flag) takes over and clears it.
  const [pending, setPending] = useState<Record<string, number>>({});

  // Optimism must YIELD to real data: clear a corpus's pending flag the
  // moment the daemon reports real indexing, or after a 15s safety net
  // (so a too-fast-to-observe reindex can never stick "Catching up…").
  useEffect(() => {
    if (!corpora) return;
    setPending((p) => {
      const next = { ...p };
      let changed = false;
      for (const { info, fresh } of corpora) {
        const started = next[info.id];
        if (started && (fresh.indexing || Date.now() - started > 15_000)) {
          delete next[info.id];
          changed = true;
        }
      }
      return changed ? next : p;
    });
  }, [corpora]);

  const rows = useMemo(() => {
    if (!corpora) return [];
    const summarized = corpora.map(({ info, fresh }) => ({
      info,
      fresh,
      summary: summarizeCounts(info.display_name, {
        stale: fresh.stale,
        new: fresh.new,
        indexing: fresh.indexing || Boolean(pending[info.id]),
      }),
    }));
    // Worst first: behind > updating > ok (mission-control exception order).
    const rank = { stale: 0, updating: 1, hidden: 2, ok: 3 } as const;
    return summarized.sort((a, b) => rank[a.summary.state] - rank[b.summary.state]);
  }, [corpora, pending]);

  // Connection states (gui-rw-daemon-down-states): boot while the very
  // first fetch is in flight; unreachable when polls fail with nothing
  // to show; degraded note when last-good data is on screen.
  // Footer is omitted on these two states: ministr is not confirmed
  // running, so the "● ministr running" trust-footer would be a lie.
  if (corpora === null && error === null) {
    return (
      <Screen align="center" gap="lg" footer={null}>
        <div className="flex flex-col items-center gap-6">
          <Brand />
          <Beat sentence="connecting to ministr…" />
        </div>
      </Screen>
    );
  }
  if (corpora === null && error !== null) {
    return (
      <Screen align="center" gap="lg" footer={null}>
        <Brand />
        <StatusBanner
          state="stale"
          headline="ministr isn’t running on this Mac"
          sub="start ministr (or restart this app) — it reconnects automatically"
        />
      </Screen>
    );
  }

  return (
    <Screen
      // A populated list anchors at the top and fills the column (it's a
      // tool, not a hero); only the genuinely-short empty state stays
      // centered (gui-ux-density-rebalance).
      align={rows.length === 0 ? "center" : "start"}
      header={
        <ShellHeader
          leading={<Brand />}
          trailing={
            <>
              {error ? <ConnectionNote /> : null}
              <SettingsMenu />
            </>
          }
        />
      }
    >
      <section className="flex flex-col gap-3" aria-label="your projects">
        {rows.map(({ info, fresh, summary }) => {
          const data: ProjectCardData = {
            name: info.display_name,
            status: summary.state,
            files: info.files_indexed,
            sections: info.sections_count,
            behind: fresh.stale + fresh.new,
            agents: info.active_sessions,
            stack: info.stack ?? [],
            symbols: info.symbols_count,
            indexedAgo: info.last_indexed ? relTime(info.last_indexed) : undefined,
            progress: progress.get(info.id),
          };
          return (
            <ProjectCard
              key={info.id}
              data={data}
              onOpen={() => onOpenProject(info)}
              onReindex={() => {
                // Optimistic: flag pending now; real indexing data takes
                // over (or the 15s net clears it). Reuses the machinery
                // CatchUp used, now driven by the card's reindex icon.
                setPending((p) => ({ ...p, [info.id]: Date.now() }));
                void triggerReindex(info.id).catch(() => {});
              }}
            />
          );
        })}
        {corpora && rows.length === 0 ? (
          <div className="space-y-3 py-12 text-center">
            <p className="text-sm text-dim">
              No projects yet — add a folder and your AI can start reading it.
            </p>
            <ActionChip variant="primary" onClick={onAddProject}>
              Choose a folder…
            </ActionChip>
          </div>
        ) : null}
        {corpora && rows.length > 0 ? (
          <div className="pt-2">
            <ActionChip onClick={onAddProject}>+ Add a project</ActionChip>
          </div>
        ) : null}
      </section>
    </Screen>
  );
}

async function fetchAll(): Promise<
  { info: CorpusInfo; fresh: FreshnessSummary }[]
> {
  const corpora = await listCorpora();
  return Promise.all(
    corpora.map(async (info) => ({
      info,
      fresh: await corpusFreshnessSummary(info.id).catch(
        (): FreshnessSummary => ({
          current: 0,
          stale: 0,
          new: 0,
          missing: 0,
          indexing: false,
        }),
      ),
    })),
  );
}
