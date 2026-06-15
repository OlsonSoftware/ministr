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
import { Screen } from "../ui/Screen";
import { useIngestionProgress } from "../../lib/useIngestionProgress";

/**
 * Home — the Trust Panel (UX-BLUEPRINT §3.1). One plain-English trust
 * row per project, worst first; healthy projects stay quiet.
 */
export function TrustPanel({
  onOpenProject,
  onAddProject,
  onOpenFeed,
}: {
  onOpenProject: (corpus: CorpusInfo) => void;
  // Required: the empty home must NEVER dead-end — the "Choose a folder…"
  // CTA is the only way out of a zero-project state, so every mount wires it.
  onAddProject: () => void;
  // Per-card "What it did" entry — makes the activity/Proof Feed
  // discoverable from Home (gui-ux-wayfinding-feed-access), not buried two
  // levels down behind the Mirror.
  onOpenFeed: (corpus: CorpusInfo) => void;
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
      align="center"
      header={
        <div className="flex items-center justify-between">
          <Brand />
          <div className="flex items-center gap-4">
            {error ? <ConnectionNote /> : null}
            <ThemePick />
          </div>
        </div>
      }
    >
      <section className="flex flex-col gap-3" aria-label="your projects">
        {rows.map(({ info, summary }) => (
          <div key={info.id} className="relative">
            <button
              type="button"
              aria-label={`open ${info.display_name}`}
              onClick={() => onOpenProject(info)}
              className="peer absolute inset-0 z-0 cursor-pointer rounded-lg focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand"
            />
            {/* Calm clickable-card affordance: the whole card lifts on
                hover AND keyboard focus (WCAG 1.4.13 parity) — a neutral
                ring + shadow-sm, never a second hue. A first-timer can see
                the row is a thing you open. */}
            <div className="rounded-lg transition peer-hover:shadow-sm peer-hover:ring-1 peer-hover:ring-dim peer-focus-visible:shadow-sm peer-focus-visible:ring-1 peer-focus-visible:ring-dim">
              <StatusBanner
                state={summary.state}
                headline={summary.headline}
                sub={`${info.display_name} · ${summary.sub}${
                  info.active_sessions > 0
                    ? ` · ${info.active_sessions} agent${info.active_sessions === 1 ? "" : "s"} connected`
                    : ""
                }`}
                action={
                  <div className="flex items-center gap-2">
                    {summary.state === "stale" ? (
                      <CatchUp
                        corpusId={info.id}
                        onAccepted={() =>
                          setPending((p) => ({ ...p, [info.id]: Date.now() }))
                        }
                      />
                    ) : null}
                    <ActionChip
                      aria-label={`what ministr did for ${info.display_name}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        onOpenFeed(info);
                      }}
                    >
                      What it did
                    </ActionChip>
                  </div>
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
          </div>
        ))}
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
