import { useEffect, useMemo, useState } from "react";
import { corpusFreshnessSummary, listCorpora } from "../../lib/ipc";
import type { CorpusInfo, FreshnessSummary } from "../../lib/ipc";
import { usePoll } from "../../lib/usePoll";
import { summarizeCounts } from "../../lib/trustSummary";
import { StatusBanner } from "../ui/StatusBanner";
import { ActionChip } from "../ui/ActionChip";
import { CatchUp } from "../ui/CatchUp";
import { Brand } from "../ui/Brand";
import { ThemePick } from "../ui/ThemePick";
import { ConnectionNote } from "../ui/ConnectionNote";
import { Beat } from "../ui/Beat";
import { IndexingInstrument } from "../ui/IndexingInstrument";
import { useIngestionProgress } from "../../lib/useIngestionProgress";

/**
 * Home — the Trust Panel (UX-BLUEPRINT §3.1). One plain-English trust
 * row per project, worst first; healthy projects stay quiet.
 */
export function TrustPanel({
  onOpenProject,
  onAddProject,
}: {
  onOpenProject: (corpus: CorpusInfo) => void;
  onAddProject?: () => void;
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
  if (corpora === null && error === null) {
    return (
      <div className="mx-auto flex min-h-screen max-w-3xl flex-col items-center justify-center gap-6 p-8">
        <Brand />
        <Beat sentence="connecting to ministr…" />
      </div>
    );
  }
  if (corpora === null && error !== null) {
    return (
      <div className="mx-auto flex min-h-screen max-w-3xl flex-col justify-center gap-6 p-8">
        <Brand />
        <StatusBanner
          state="stale"
          headline="ministr isn’t running on this Mac"
          sub="start ministr (or restart this app) — it reconnects automatically"
        />
      </div>
    );
  }

  return (
    <div className="mx-auto flex min-h-screen max-w-3xl flex-col gap-4 p-8">
      <header className="flex items-center justify-between">
        <Brand />
        <div className="flex items-center gap-4">
          {error ? <ConnectionNote /> : null}
          <ThemePick />
        </div>
      </header>

      <main className="flex flex-col gap-3" aria-label="your projects">
        {rows.map(({ info, summary }) => (
          <div key={info.id} className="relative">
            <button
              type="button"
              aria-label={`open ${info.display_name}`}
              onClick={() => onOpenProject(info)}
              className="absolute inset-0 z-0 rounded-lg focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand"
            />
            <StatusBanner
              state={summary.state}
              headline={summary.headline}
              sub={`${info.display_name} · ${summary.sub}${
                info.active_sessions > 0
                  ? ` · ${info.active_sessions} agent${info.active_sessions === 1 ? "" : "s"} connected`
                  : ""
              }`}
              action={
                summary.state === "stale" ? (
                  <CatchUp
                    corpusId={info.id}
                    onAccepted={() =>
                      setPending((p) => ({ ...p, [info.id]: Date.now() }))
                    }
                  />
                ) : undefined
              }
              footer={
                summary.state === "updating" &&
                progress.get(info.id)?.running ? (
                  <IndexingInstrument
                    progress={progress.get(info.id)!}
                    variant="compact"
                  />
                ) : undefined
              }
            />
          </div>
        ))}
        {corpora && rows.length === 0 ? (
          <div className="space-y-3 py-12 text-center">
            <p className="text-sm text-dim">
              No projects yet — add a folder and your AI can start reading it.
            </p>
            {onAddProject ? (
              <ActionChip variant="primary" onClick={onAddProject}>
                Add a project
              </ActionChip>
            ) : null}
          </div>
        ) : null}
        {corpora && rows.length > 0 && onAddProject ? (
          <div className="pt-2">
            <ActionChip onClick={onAddProject}>+ Add a project</ActionChip>
          </div>
        ) : null}
      </main>
    </div>
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
