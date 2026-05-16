import { useMemo } from "react";
import type { CoherenceEvent, CoherenceKind } from "../../lib/types";
import { cn } from "../../lib/utils";
import { relative } from "../../lib/time";

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

const KIND_BORDER: Record<CoherenceKind, string> = {
  created: "border-l-success",
  modified: "border-l-accent",
  removed: "border-l-danger",
};

const KIND_BADGE_BG: Record<CoherenceKind, string> = {
  created: "bg-success",
  modified: "bg-accent",
  removed: "bg-danger",
};

const KIND_TEXT: Record<CoherenceKind, string> = {
  created: "text-success",
  modified: "text-accent",
  removed: "text-danger",
};

function shortPath(path: string): string {
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
          "flex items-center justify-center rounded-lg border border-dashed border-border bg-surface px-4 py-8 text-mono-mini font-mono uppercase tracking-[0.08em] text-text-dim",
          className,
        )}
      >
        No file changes observed
      </div>
    );
  }

  return (
    <ul className={cn("flex flex-col gap-1", className)}>
      {filtered.map((ev) => {
        const fresh =
          typeof flashSince === "number" && ev.timestamp_ms > flashSince;
        const glyph = KIND_GLYPH[ev.kind];
        const sectionCount = ev.affected_sections.length;

        return (
          <li
            key={`${ev.timestamp_ms}-${ev.path}-${ev.kind}`}
            className={cn(
              "flex items-center gap-2 rounded-md border border-l-2 border-border bg-surface pl-2 pr-2 py-1.5 text-mono-mini",
              KIND_BORDER[ev.kind],
              fresh && "ministr-pulse",
            )}
          >
            <span
              aria-hidden
              className={cn(
                "inline-flex h-5 w-5 shrink-0 items-center justify-center border border-border-soft font-mono text-xs font-bold text-[var(--color-accent-fg-on)]",
                KIND_BADGE_BG[ev.kind],
              )}
            >
              {glyph}
            </span>

            <span
              className={cn(
                "font-mono text-xs font-semibold uppercase tracking-[0.08em]",
                KIND_TEXT[ev.kind],
              )}
            >
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
                className="font-mono text-xs tabular-nums rounded-full border border-warning/40 bg-surface px-2 py-0 text-warning"
                title={
                  sectionCount === 1
                    ? "1 section invalidated"
                    : `${sectionCount} sections invalidated`
                }
              >
                {sectionCount}§
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
