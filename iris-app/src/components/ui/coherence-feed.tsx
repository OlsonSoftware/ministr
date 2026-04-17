import { useMemo } from "react";
import type { CoherenceEvent, CoherenceKind } from "../../lib/types";
import { cn } from "../../lib/utils";

interface CoherenceFeedProps {
  events: CoherenceEvent[];
  /** Max rows to render. Defaults to 12. */
  limit?: number;
  /** Timestamp (ms) of the previous snapshot — rows newer than this flash in. */
  flashSince?: number;
  /** Filter to a specific corpus id. */
  corpusId?: string;
  className?: string;
}

const KIND_GLYPH: Record<CoherenceKind, string> = {
  created: "+",
  modified: "~",
  removed: "−",
};

const KIND_TONE: Record<
  CoherenceKind,
  {
    border: string;
    badge: string;
    text: string;
  }
> = {
  created: {
    border: "border-l-success/70",
    badge: "bg-success/15 text-success",
    text: "text-success",
  },
  modified: {
    border: "border-l-accent/70",
    badge: "bg-accent/15 text-accent",
    text: "text-accent",
  },
  removed: {
    border: "border-l-danger/70",
    badge: "bg-danger/15 text-danger",
    text: "text-danger",
  },
};

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

function shortPath(path: string): string {
  // Drop everything before the last 2 path segments so rows fit.
  const parts = path.split("/");
  if (parts.length <= 2) return path;
  return `…/${parts.slice(-2).join("/")}`;
}

export function CoherenceFeed({
  events,
  limit = 12,
  flashSince,
  corpusId,
  className,
}: CoherenceFeedProps) {
  const now = Date.now();
  const filtered = useMemo(() => {
    const rows = corpusId
      ? events.filter((e) => e.corpus_id === corpusId)
      : events;
    return rows.slice(0, limit);
  }, [events, limit, corpusId]);

  if (filtered.length === 0) {
    return (
      <div
        className={cn(
          "flex items-center justify-center rounded-lg border border-dashed border-border/50 bg-surface-raised/30 px-4 py-8 text-[11px] text-text-dim",
          className,
        )}
      >
        No file changes observed — edits will stream here live.
      </div>
    );
  }

  return (
    <ul className={cn("flex flex-col gap-1", className)}>
      {filtered.map((ev) => {
        const fresh =
          typeof flashSince === "number" && ev.timestamp_ms > flashSince;
        const tone = KIND_TONE[ev.kind];
        const glyph = KIND_GLYPH[ev.kind];
        const sectionCount = ev.affected_sections.length;

        return (
          <li
            key={`${ev.timestamp_ms}-${ev.path}-${ev.kind}`}
            className={cn(
              "flex items-center gap-2 rounded-md border-l-2 bg-surface-raised/30 pl-2 pr-2 py-1.5 text-[11px] transition-colors",
              tone.border,
              fresh && "iris-flash",
            )}
          >
            <span
              aria-hidden
              className={cn(
                "inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-sm font-mono text-[12px] font-bold",
                tone.badge,
              )}
            >
              {glyph}
            </span>

            <span className={cn("font-mono text-[10px] uppercase tracking-wider", tone.text)}>
              {ev.kind}
            </span>

            <span
              className="font-mono text-text-muted truncate flex-1"
              title={ev.path}
            >
              {shortPath(ev.path)}
            </span>

            {sectionCount > 0 ? (
              <span
                className="font-mono text-[10px] tabular-nums rounded-sm bg-warning/10 px-1.5 py-0.5 text-warning"
                title={
                  sectionCount === 1
                    ? "1 section invalidated"
                    : `${sectionCount} sections invalidated`
                }
              >
                {sectionCount}§
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
