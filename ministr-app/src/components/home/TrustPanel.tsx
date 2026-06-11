import { useMemo } from "react";
import { corpusFreshness, listCorpora, triggerReindex } from "../../lib/ipc";
import type { CorpusInfo, FreshnessResponse } from "../../lib/ipc";
import { usePoll } from "../../lib/usePoll";
import { summarize } from "../../lib/trustSummary";
import { StatusBanner } from "../ui/StatusBanner";
import { ActionChip } from "../ui/ActionChip";
import { Brand } from "../ui/Brand";

/**
 * Home — the Trust Panel (UX-BLUEPRINT §3.1). One plain-English trust
 * row per project, worst first; healthy projects stay quiet.
 */
export function TrustPanel({
  onOpenProject,
}: {
  onOpenProject: (corpus: CorpusInfo) => void;
}) {
  const { data: corpora, error } = usePoll(fetchAll, 5_000);

  const rows = useMemo(() => {
    if (!corpora) return [];
    const summarized = corpora.map(({ info, fresh }) => ({
      info,
      summary: summarize(info.display_name, fresh),
    }));
    // Worst first: behind > updating > ok (mission-control exception order).
    const rank = { stale: 0, updating: 1, hidden: 2, ok: 3 } as const;
    return summarized.sort((a, b) => rank[a.summary.state] - rank[b.summary.state]);
  }, [corpora]);

  return (
    <div className="mx-auto flex min-h-screen max-w-3xl flex-col gap-4 p-8">
      <header className="flex items-center justify-between">
        <Brand />
        {error ? (
          <span className="text-sm text-dim">can’t reach ministr right now</span>
        ) : null}
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
                  ? ` · ${info.active_sessions} agent${info.active_sessions === 1 ? "" : "s"} reading`
                  : ""
              }`}
              action={
                summary.state === "stale" ? (
                  <ActionChip
                    variant="primary"
                    onClick={(e) => {
                      e.stopPropagation();
                      void triggerReindex(info.id);
                    }}
                  >
                    Catch up
                  </ActionChip>
                ) : undefined
              }
            />
          </div>
        ))}
        {corpora && rows.length === 0 ? (
          <p className="py-12 text-center text-sm text-dim">
            No projects yet — add a folder and your AI can start reading it.
          </p>
        ) : null}
      </main>
    </div>
  );
}

async function fetchAll(): Promise<
  { info: CorpusInfo; fresh: FreshnessResponse }[]
> {
  const corpora = await listCorpora();
  return Promise.all(
    corpora.map(async (info) => ({
      info,
      fresh: await corpusFreshness(info.id).catch(
        (): FreshnessResponse => ({ files: [], indexing: false }),
      ),
    })),
  );
}
