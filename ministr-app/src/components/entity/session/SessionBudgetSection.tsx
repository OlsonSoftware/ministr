import { useMemo } from "react";
import type { SessionDetail } from "../../../lib/types";
import { formatTokens } from "../../../lib/format";
import {
  burnRate,
  type SessionSample,
  thresholdsFor,
  utilizationTone,
} from "../../../lib/sessions";
import { EntitySection, EntitySectionEmpty } from "../EntitySection";
import { Sparkline } from "../../ui/sparkline";

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
 * §Budget Trend — trajectory beats snapshot for a live tool. Sparklines
 * are drawn from the client-side poll-sampled ring (the daemon keeps no
 * per-session series); they are stepped, not smoothed (brutalist).
 */
export function SessionBudgetSection({
  chapter,
  session,
  samples,
  ended,
}: Props) {
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

  if (samples.length < 2) {
    return (
      <EntitySection chapter={chapter} title="Budget trend" empty>
        <EntitySectionEmpty
          label={
            ended
              ? "No live trend — session is a historical snapshot."
              : "Collecting samples — the trend appears as the session runs."
          }
        />
      </EntitySection>
    );
  }

  const spanMin = Math.max(
    1,
    Math.round((samples[samples.length - 1].t - samples[0].t) / 60000),
  );

  return (
    <EntitySection
      chapter={chapter}
      title="Budget trend"
      meta={burnLabel(burn.tokensPerSec)}
    >
      <div className="px-3 py-3 space-y-3">
        <div className="space-y-1">
          <div className="flex items-baseline justify-between">
            <span className="font-mono text-mono-mini uppercase tracking-[0.05em] text-text-dim">
              Tokens in context
            </span>
            <span className="font-mono text-xs tabular-nums text-text">
              {formatTokens(session.tokens_used)}
            </span>
          </div>
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
          <span className="font-mono text-mono-mini uppercase tracking-[0.05em] text-text-dim">
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
        <p className="font-serif text-xs italic text-text-dim leading-snug">
          ~{spanMin}m window · {samples.length} samples · sampled on each poll
        </p>
      </div>
    </EntitySection>
  );
}
