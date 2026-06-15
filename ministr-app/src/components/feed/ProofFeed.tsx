import { useMemo } from "react";
import { corpusOutcomes, recentActivity } from "../../lib/ipc";
import type { ActivityEvent, CorpusInfo, OutcomesResponse } from "../../lib/ipc";
import { usePoll } from "../../lib/usePoll";
import { ConnectionNote } from "../ui/ConnectionNote";
import { aggregate, buildFeed, clock } from "../../lib/receipts";
import { Receipt } from "../ui/Receipt";
import { BackButton } from "../ui/BackButton";
import { Screen } from "../ui/Screen";

/**
 * Proof Feed — the trust-evidence engine (UX-BLUEPRINT §3.3).
 * Every line restates one recorded event; wins and heads-ups share
 * identical typography (DESIGN §2.3); the bottom line is counts only.
 * The expert ledger (raw tool calls) lives one disclosure down.
 */
export function ProofFeed({
  corpus,
  onBack,
  backLabel = "Back",
}: {
  corpus: CorpusInfo;
  onBack: () => void;
  /** Where the back affordance returns to, e.g. "All projects". */
  backLabel?: string;
}) {
  const { data, error: connError } = usePoll(() => fetchFeed(corpus.id), 4_000);

  const lines = useMemo(
    () => (data ? buildFeed(data.activity, data.outcomes.events) : []),
    [data],
  );

  return (
    <Screen
      align="start"
      header={
        <div className="flex items-center gap-3">
          <BackButton onClick={onBack} label={backLabel} />
          <h1 className="text-xl font-semibold tracking-tight text-ink">
            {corpus.display_name}
            <span className="ml-2 text-sm font-normal text-dim">
              what ministr did for your AI
            </span>
          </h1>
        </div>
      }
    >
      {connError && data ? <ConnectionNote /> : null}

      <section
        aria-label="receipts"
        className="divide-y divide-line rounded-lg border border-line bg-surface p-1"
      >
        {lines.map((l, i) => (
          <Receipt
            key={`${l.ts}-${i}`}
            time={clock(l.ts)}
            sentence={l.sentence}
            kind={l.kind}
          />
        ))}
        {data && lines.length === 0 ? (
          <p className="p-4 text-sm text-dim">
            nothing yet — receipts appear as your AI works
          </p>
        ) : null}
      </section>

      {data && lines.length > 0 ? (
        <p className="px-2 text-sm text-dim">
          {aggregate(data.activity, data.outcomes)}
        </p>
      ) : null}

      <details className="px-2">
        <summary className="cursor-pointer text-xs text-dim">
          expert view — raw tool calls
        </summary>
        <div className="mt-2 rounded-lg border border-line bg-sunken p-2">
          {data?.activity.map((e, i) => (
            <div
              key={`${e.timestamp_ms}-${i}`}
              className="flex items-baseline gap-3 px-1 py-0.5 font-mono text-xs text-dim"
            >
              <span>{clock(e.timestamp_ms)}</span>
              <span className="text-ink">{e.tool}</span>
              <span className="min-w-0 flex-1 truncate">{e.summary}</span>
              {e.cache_hit ? <span>cache</span> : null}
              {typeof e.tokens_delta === "number" ? (
                <span>{e.tokens_delta}tok</span>
              ) : null}
            </div>
          ))}
        </div>
      </details>
    </Screen>
  );
}

async function fetchFeed(corpusId: string): Promise<{
  activity: ActivityEvent[];
  outcomes: OutcomesResponse;
}> {
  const [activityAll, outcomes] = await Promise.all([
    recentActivity(100),
    corpusOutcomes(corpusId).catch(
      (): OutcomesResponse => ({ events: [], stats: [] }),
    ),
  ]);
  return {
    activity: activityAll.filter((e) => e.corpus_id === corpusId),
    outcomes,
  };
}
