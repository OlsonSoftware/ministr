import { useMemo } from "react";
import type { ActivityEvent } from "../../lib/types";
import { cn } from "../../lib/utils";

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
  normal: "border-l-border/40",
  elevated: "border-l-warning/70",
  critical: "border-l-danger",
};

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toString();
}

function relative(nowMs: number, tsMs: number): string {
  const delta = Math.max(0, nowMs - tsMs);
  const secs = Math.floor(delta / 1000);
  if (secs < 1) return "now";
  if (secs < 60) return `${secs}s ago`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  return `${hrs}h ago`;
}

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
          "flex items-center justify-center rounded-lg border border-dashed border-border/50 bg-surface-raised/30 px-4 py-10 text-[11px] text-text-dim",
          className,
        )}
      >
        No tool activity yet — agent calls will stream here live.
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
            : "border-l-transparent";

        return (
          <li
            key={`${ev.timestamp_ms}-${ev.tool}-${ev.corpus_id}`}
            className={cn(
              "flex items-center gap-2 rounded-md border-l-2 bg-surface-raised/30 pl-2 pr-2 py-1.5 text-[11px] transition-colors",
              pressureBorder,
              fresh && "ministr-flash",
            )}
          >
            <span
              className={cn(
                "inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-sm font-mono text-[12px]",
                ev.cache_hit
                  ? "bg-success/15 text-success"
                  : "bg-accent/15 text-accent",
              )}
              aria-hidden
            >
              {glyph}
            </span>

            <span className="font-mono text-text whitespace-nowrap">
              {ev.tool.replace(/^ministr_/, "")}
            </span>

            <span className="text-text-dim truncate flex-1">
              {ev.summary || ev.corpus_id}
            </span>

            {ev.cache_hit ? (
              <span className="font-mono text-[10px] uppercase tracking-wider rounded-sm bg-success/15 px-1.5 py-0.5 text-success">
                hit
              </span>
            ) : typeof ev.tokens_delta === "number" && ev.tokens_delta > 0 ? (
              <span className="font-mono text-[10px] tabular-nums rounded-sm bg-accent/10 px-1.5 py-0.5 text-accent">
                +{formatTokens(ev.tokens_delta)}
              </span>
            ) : null}

            <span className="font-mono text-[10px] text-text-dim tabular-nums whitespace-nowrap w-14 text-right">
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
 * compute the cache-hit ratio per bucket. Used to drive the real-history
 * hit-rate bars in Overview.
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
    // Oldest bucket = 0, newest = bucketCount - 1.
    const idx = Math.min(
      bucketCount - 1,
      Math.max(0, bucketCount - 1 - Math.floor(age / bucketSize)),
    );
    const [total, hits] = buckets[idx];
    buckets[idx] = [total + 1, hits + (ev.cache_hit ? 1 : 0)];
  }

  return buckets.map(([total, hits]) => (total === 0 ? 0 : hits / total));
}
