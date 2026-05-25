import { useMemo } from "react";
import type { SessionDetail } from "../../../lib/types";
import { formatTokens } from "../../../lib/format";
import {
  burnRate,
  deriveVitals,
  type SessionSample,
  thresholdsFor,
  utilizationTone,
} from "../../../lib/sessions";
import { Disclosure } from "../../ui/disclosure";
import { Sparkline } from "../../ui/sparkline";
import { TokenEconomicsBar } from "../../ui/token-economics-bar";

interface Props {
  chapter: number;
  session: SessionDetail;
  samples: readonly SessionSample[];
  ended: boolean;
}

function burnLabel(tokensPerSec: number | null): string {
  if (tokensPerSec == null || tokensPerSec <= 0) return "stable";
  return `≈ ${formatTokens(Math.round(tokensPerSec * 60))}/min`;
}

/**
 * §Token usage — the agent-context machinery (delivered/saved/live bar +
 * burn-rate trend + reclaimed counters). Demoted from a headline section
 * to a collapsed `<details>` card under the new code-intelligence
 * framing — the data stays one click away for power users without
 * dominating the panel.
 */
export function SessionTokenAdvanced({
  chapter,
  session,
  samples,
  ended,
}: Props) {
  const v = deriveVitals(session);
  const thresholds = thresholdsFor(session);
  const tokenSeries = useMemo(
    () => samples.map((s) => s.tokensUsed),
    [samples],
  );
  const pressureTones = useMemo(
    () => samples.map((s) => utilizationTone(s.utilization, thresholds)),
    [samples, thresholds],
  );
  const burn = burnRate(samples);

  // Compact one-line header summary — what you see without expanding.
  const headerMeta = v ? (
    <span className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
      <span className="whitespace-nowrap">{v.pct}%</span>
      <span aria-hidden>·</span>
      <span className="whitespace-nowrap">
        {formatTokens(session.cumulative_tokens_delivered)} served
      </span>
      <span aria-hidden>·</span>
      <span className="whitespace-nowrap">
        {formatTokens(v.tokensSaved)} saved
      </span>
    </span>
  ) : null;

  if (!v) return null;

  const evicted = session.cumulative_tokens_evicted;
  const compacted = session.cumulative_tokens_compressed;
  const hasTokenSplit =
    typeof evicted === "number" && typeof compacted === "number";
  const hasTrend = samples.length >= 2;
  const spanMin = hasTrend
    ? Math.max(
        1,
        Math.round((samples[samples.length - 1].t - samples[0].t) / 60000),
      )
    : 0;

  return (
    <Disclosure chapter={chapter} title="Token usage" meta={headerMeta}>
      {/* Bar */}
      <div className="px-3 py-3">
        <TokenEconomicsBar
          deliveredTokens={session.cumulative_tokens_delivered}
          savedTokens={v.tokensSaved}
          liveTokens={v.tokensUsed}
        />
      </div>

      {/* Reclaimed footer */}
      <p className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5 border-t border-border-soft px-3 py-2 font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
        <span className="whitespace-nowrap">Reclaimed</span>
        <span aria-hidden>·</span>
        <span className="whitespace-nowrap text-text">
          {hasTokenSplit
            ? `${formatTokens(evicted ?? 0)} evicted`
            : `${v.evictions.toLocaleString()}× evicted`}
        </span>
        <span aria-hidden>·</span>
        <span className="whitespace-nowrap text-text">
          {hasTokenSplit
            ? `${formatTokens(compacted ?? 0)} compacted`
            : `${v.compressions.toLocaleString()}× compacted`}
        </span>
        {v.cacheHitRate != null && (
          <>
            <span aria-hidden>·</span>
            <span className="whitespace-nowrap">
              <span>cache hit </span>
              <span className="text-success">
                {Math.round(v.cacheHitRate * 100)}%
              </span>
            </span>
          </>
        )}
        {typeof session.delta_updates === "number" &&
          session.delta_updates > 0 && (
            <>
              <span aria-hidden>·</span>
              <span className="whitespace-nowrap text-text">
                {session.delta_updates.toLocaleString()} Δ updates
              </span>
            </>
          )}
      </p>

      {/* Trend */}
      <div className="border-t border-border-soft px-3 py-3 space-y-3">
        <div className="flex items-baseline justify-between">
          <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
            Budget trend
          </span>
          <span className="font-mono text-xs tabular-nums text-text-dim shrink-0">
            {burnLabel(burn.tokensPerSec)}
          </span>
        </div>
        {hasTrend ? (
          <>
            <div className="space-y-1">
              <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
                Tokens in context
              </span>
              <div className="border border-border-soft bg-surface-sunken">
                <Sparkline
                  data={tokenSeries}
                  mode="line"
                  tone="accent"
                  height={44}
                  ariaLabel={`Tokens in context over time, ${samples.length} samples, currently ${formatTokens(
                    session.tokens_used,
                  )}`}
                />
              </div>
            </div>
            <div className="space-y-1">
              <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
                Headroom
              </span>
              <div className="border border-border-soft">
                <Sparkline
                  data={tokenSeries}
                  mode="band"
                  bandTones={pressureTones}
                  height={10}
                  ariaLabel="Budget pressure over time"
                />
              </div>
            </div>
            <p className="font-sans text-xs text-text-dim leading-snug">
              ~{spanMin}m window · {samples.length} samples · sampled on each
              poll
            </p>
          </>
        ) : (
          <p className="font-sans text-sm text-text-dim leading-snug">
            {ended
              ? "No live trend — session is a historical snapshot."
              : "Collecting samples — the trend appears as the session runs."}
          </p>
        )}
      </div>
    </Disclosure>
  );
}
