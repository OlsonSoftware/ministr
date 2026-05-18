import { formatTokens } from "../../lib/format";
import { labelMicro } from "../../lib/ui-tokens";
import { cn } from "../../lib/utils";

interface TokenEconomicsBarProps {
  /** `cumulative_tokens_delivered` — everything ministr has served. */
  deliveredTokens: number;
  /** `total_tokens_saved` — evicted + compacted (never re-sent). */
  savedTokens: number;
  /** `tokens_used` — what is live in the window right now. */
  liveTokens: number;
  className?: string;
}

interface Seg {
  label: string;
  tokens: number;
  /** Tailwind bg utility for the bar segment + legend swatch. */
  bg: string;
}

/**
 * The token-economics story in one stacked bar: what ministr served vs
 * what it saved you vs what is live. Brutalist — solid segments, hard 2px
 * dividers, sharp corners, no gradient. The mono legend carries the
 * numbers (the bar is the glanceable proportion).
 */
export function TokenEconomicsBar({
  deliveredTokens,
  savedTokens,
  liveTokens,
  className,
}: TokenEconomicsBarProps) {
  const segs: Seg[] = [
    { label: "Delivered", tokens: deliveredTokens, bg: "bg-accent" },
    { label: "Saved", tokens: savedTokens, bg: "bg-success" },
    { label: "Live", tokens: liveTokens, bg: "bg-text-dim" },
  ];
  const total = segs.reduce((a, s) => a + s.tokens, 0);

  return (
    <div className={cn("space-y-2", className)}>
      <div className="flex h-6 w-full overflow-hidden rounded-full border border-border bg-surface-overlay">
        {total > 0 &&
          segs.map((s, i) => (
            <div
              key={s.label}
              className={cn(
                "h-full",
                s.bg,
                i < segs.length - 1 && "border-r border-border",
              )}
              style={{ flexBasis: `${(s.tokens / total) * 100}%` }}
            />
          ))}
      </div>
      <div className="flex flex-wrap gap-x-4 gap-y-1">
        {segs.map((s) => (
          <div
            key={s.label}
            className="flex items-center gap-1.5 whitespace-nowrap"
          >
            <span
              className={cn("h-2.5 w-2.5 rounded-full border border-border", s.bg)}
              aria-hidden="true"
            />
            <span className={labelMicro}>{s.label}</span>
            <span className="font-mono text-mono-mini tabular-nums text-text">
              {formatTokens(s.tokens)}
            </span>
            <span className="font-mono text-mono-mini tabular-nums text-text-dim">
              {total > 0 ? `${Math.round((s.tokens / total) * 100)}%` : "—"}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
