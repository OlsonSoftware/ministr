import { useMemo } from "react";
import type { ActivityEvent } from "../../lib/types";
import { cn } from "../../lib/utils";
import { formatTokens } from "../../lib/format";
import { relative } from "../../lib/time";

interface ActivityFeedProps {
  events: ActivityEvent[];
  /** Max rows to render. Defaults to 20. */
  limit?: number;
  /** Timestamp (ms) of the previous snapshot — rows newer than this flash in. */
  flashSince?: number;
  /** Filter to a specific session id. */
  sessionId?: string;
  className?: string;
}

const TOOL_GLYPH: Record<string, string> = {
  ministr_survey: "⏺",
  ministr_read: "⎈",
  ministr_symbols: "⌺",
  ministr_references: "▣",
  ministr_definition: "⎔",
  ministr_extract: "✦",
  ministr_related: "⟐",
  ministr_toc: "◇",
  ministr_budget: "◑",
  ministr_compress: "▼",
  ministr_evicted: "✕",
  ministr_bridge: "⟁",
  ministr_fetch: "↓",
  ministr_clone: "⎋",
  ministr_refresh: "↻",
  ministr_ask: "?",
};

const PRESSURE_BORDER: Record<string, string> = {
  normal: "border-l-border",
  elevated: "border-l-warning",
  critical: "border-l-danger",
};

export function ActivityFeed({
  events,
  limit = 20,
  flashSince,
  sessionId,
  className,
}: ActivityFeedProps) {
  const now = Date.now();
  const filtered = useMemo(() => {
    const rows = sessionId
      ? events.filter((e) => e.session_id === sessionId)
      : events;
    return rows.slice(0, limit);
  }, [events, limit, sessionId]);

  if (filtered.length === 0) {
    return (
      <div
        className={cn(
          "flex items-center justify-center rounded-lg border border-dashed border-border bg-surface px-4 py-10 text-mono-mini font-mono uppercase tracking-[0.08em] text-text-dim",
          className,
        )}
      >
        No tool activity yet
      </div>
    );
  }

  return (
    <ul className={cn("flex flex-col gap-1", className)}>
      {filtered.map((ev) => {
        const fresh =
          typeof flashSince === "number" && ev.timestamp_ms > flashSince;
        const glyph = TOOL_GLYPH[ev.tool] ?? "•";
        const pressureBorder =
          ev.pressure && PRESSURE_BORDER[ev.pressure]
            ? PRESSURE_BORDER[ev.pressure]
            : "border-l-border";

        return (
          <li
            key={`${ev.timestamp_ms}-${ev.tool}-${ev.corpus_id}`}
            className={cn(
              "flex items-center gap-2 rounded-md border border-l-2 border-border bg-surface pl-2 pr-2 py-1.5 text-mono-mini",
              pressureBorder,
              fresh && "ministr-pulse",
            )}
          >
            <span
              className={cn(
                "inline-flex h-5 w-5 shrink-0 items-center justify-center border border-border-soft font-mono text-xs",
                ev.cache_hit
                  ? "bg-success text-[var(--color-accent-fg-on)]"
                  : "bg-accent text-[var(--color-accent-fg-on)]",
              )}
              aria-hidden
            >
              {glyph}
            </span>

            <span className="font-mono font-semibold text-text whitespace-nowrap">
              {ev.tool.replace(/^ministr_/, "")}
            </span>

            <span className="text-text-muted truncate flex-1 font-mono">
              {ev.summary || ev.corpus_id}
            </span>

            {ev.cache_hit ? (
              <span className="font-mono text-xs uppercase tracking-[0.08em] rounded-full border border-success/40 bg-surface px-2 py-0 text-success">
                hit
              </span>
            ) : typeof ev.tokens_delta === "number" && ev.tokens_delta > 0 ? (
              <span className="font-mono text-xs tabular-nums rounded-full border border-accent/40 bg-surface px-2 py-0 text-accent">
                +{formatTokens(ev.tokens_delta)}
              </span>
            ) : null}

            <span className="font-mono text-xs text-text-dim tabular-nums whitespace-nowrap w-14 text-right">
              {relative(now, ev.timestamp_ms)}
            </span>
          </li>
        );
      })}
    </ul>
  );
}

/**
 * Bucket activity events into `bucketCount` equally-sized time windows and
 * compute the cache-hit ratio per bucket.
 */
export function computeHitRateBuckets(
  events: ActivityEvent[],
  bucketCount: number,
  windowMs: number,
): number[] {
  if (events.length === 0) return new Array(bucketCount).fill(0);
  const now = Date.now();
  const bucketSize = windowMs / bucketCount;
  const buckets = new Array<[number, number]>(bucketCount)
    .fill([0, 0])
    .map(() => [0, 0] as [number, number]);

  for (const ev of events) {
    const age = now - ev.timestamp_ms;
    if (age < 0 || age > windowMs) continue;
    const idx = Math.min(
      bucketCount - 1,
      Math.max(0, bucketCount - 1 - Math.floor(age / bucketSize)),
    );
    const [total, hits] = buckets[idx];
    buckets[idx] = [total + 1, hits + (ev.cache_hit ? 1 : 0)];
  }

  return buckets.map(([total, hits]) => (total === 0 ? 0 : hits / total));
}
