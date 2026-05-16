import type { SessionDetail } from "../../../lib/types";
import { formatTokens } from "../../../lib/format";
import { deriveVitals } from "../../../lib/sessions";
import { EntitySection } from "../EntitySection";
import { MetricTile } from "../../ui/metric-tile";
import { TokenEconomicsBar } from "../../ui/token-economics-bar";

interface Props {
  chapter: number;
  session: SessionDetail;
}

function pctLabel(ratio: number | null): string {
  return ratio == null ? "—" : `${Math.round(ratio * 100)}%`;
}

/**
 * §Economics — the "is ministr earning its keep" story. Surfaces the
 * counters the old panel never showed: what was served vs saved vs live,
 * the evicted/compacted split, savings + cache-hit rates.
 */
export function SessionEconomicsSection({ chapter, session }: Props) {
  const v = deriveVitals(session);
  if (!v) return null;
  const evicted = session.cumulative_tokens_evicted;
  const compacted = session.cumulative_tokens_compressed;
  const hasTokenSplit =
    typeof evicted === "number" && typeof compacted === "number";

  return (
    <EntitySection
      chapter={chapter}
      title="Economics"
      meta={`+${formatTokens(v.tokensSaved)} saved`}
    >
      <div className="px-3 py-3">
        <TokenEconomicsBar
          deliveredTokens={session.cumulative_tokens_delivered}
          savedTokens={v.tokensSaved}
          liveTokens={v.tokensUsed}
        />
      </div>
      <div className="grid grid-cols-2 border-t border-border-soft divide-x divide-y divide-border-soft">
        <MetricTile
          variant="cell"
          label="Served"
          value={formatTokens(session.cumulative_tokens_delivered)}
        />
        <MetricTile
          variant="cell"
          label="Saved"
          value={formatTokens(v.tokensSaved)}
          tone="success"
        />
        <MetricTile
          variant="cell"
          label="Repeats avoided"
          value={v.dedupHits.toLocaleString()}
          tone="accent"
        />
        <MetricTile
          variant="cell"
          label="Savings rate"
          value={pctLabel(v.savingsRate)}
          tone={v.savingsRate ? "success" : undefined}
        />
      </div>
      <p className="border-t border-border-soft px-3 py-2 font-mono text-mono-mini uppercase tracking-[0.05em] text-text-dim">
        Reclaimed ·{" "}
        <span className="text-text">
          {hasTokenSplit
            ? `${formatTokens(evicted ?? 0)} evicted`
            : `${v.evictions.toLocaleString()}× evicted`}
        </span>{" "}
        ·{" "}
        <span className="text-text">
          {hasTokenSplit
            ? `${formatTokens(compacted ?? 0)} compacted`
            : `${v.compressions.toLocaleString()}× compacted`}
        </span>
        {v.cacheHitRate != null && (
          <>
            {" "}
            · cache hit{" "}
            <span className="text-success">{pctLabel(v.cacheHitRate)}</span>
          </>
        )}
      </p>
    </EntitySection>
  );
}
