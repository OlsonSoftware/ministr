import { type ReactNode, useMemo, useState } from "react";
import type { ActivityEvent } from "../../../lib/types";
import { formatTokens } from "../../../lib/format";
import { relative } from "../../../lib/time";
import { cn } from "../../../lib/utils";
import {
  EntitySection,
  EntitySectionEmpty,
  EntitySectionLoading,
} from "../EntitySection";

interface Props {
  chapter: number;
  /** Full session-scoped list, newest first. */
  events: ActivityEvent[];
  loading: boolean;
  error: string | null;
  /** Events newer than this just arrived → flash. */
  flashSince: number;
}

const PAGE = 50;

const PRESSURE_CHIP: Record<string, string> = {
  elevated: "text-warning",
  critical: "text-danger",
};

function tag(tool: string): string {
  return tool.replace(/^ministr_/, "").toUpperCase();
}

function absTime(ms: number): string {
  return new Date(ms).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

/**
 * §Activity — the work log. Filterable by tool + cache/pressure, with
 * absolute & relative time, resolution/cache chips, latency, and a
 * Δ-token bar. Progressive reveal (the ring is bounded, no cursor).
 */
export function ActivityTimeline({
  chapter,
  events,
  loading,
  error,
  flashSince,
}: Props) {
  const [tools, setTools] = useState<Set<string>>(new Set());
  const [cacheOnly, setCacheOnly] = useState(false);
  const [pressureOnly, setPressureOnly] = useState(false);
  const [limit, setLimit] = useState(PAGE);

  const allTools = useMemo(
    () => Array.from(new Set(events.map((e) => tag(e.tool)))).sort(),
    [events],
  );

  const filtered = useMemo(
    () =>
      events.filter(
        (e) =>
          (tools.size === 0 || tools.has(tag(e.tool))) &&
          (!cacheOnly || e.cache_hit) &&
          (!pressureOnly ||
            e.pressure === "elevated" ||
            e.pressure === "critical"),
      ),
    [events, tools, cacheOnly, pressureOnly],
  );

  const maxDelta = useMemo(
    () =>
      filtered.reduce((m, e) => Math.max(m, e.tokens_delta ?? 0), 0) || 1,
    [filtered],
  );

  const toggleTool = (t: string) =>
    setTools((prev) => {
      const next = new Set(prev);
      if (next.has(t)) next.delete(t);
      else next.add(t);
      return next;
    });

  const now = Date.now();
  const visible = filtered.slice(0, limit);

  let body: ReactNode;
  if (loading && events.length === 0) {
    body = <EntitySectionLoading />;
  } else if (error && events.length === 0) {
    body = <EntitySectionEmpty label={`Couldn't load activity — ${error}`} />;
  } else if (events.length === 0) {
    body = (
      <EntitySectionEmpty label="No recorded activity for this session." />
    );
  } else if (filtered.length === 0) {
    body = <EntitySectionEmpty label="No events match the filter." />;
  } else {
    body = (
      <>
        <div
          role="region"
          aria-label="Session activity"
          tabIndex={0}
          className="max-h-80 overflow-y-auto focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
        >
          {visible.map((e) => {
            const fresh = e.timestamp_ms > flashSince;
            const delta = e.tokens_delta ?? 0;
            return (
              <div
                key={`${e.timestamp_ms}-${e.tool}-${e.corpus_id}`}
                className={cn(
                  "flex flex-col gap-1 border-b border-border-soft last:border-b-0 px-3 py-2",
                  fresh && "ministr-flash",
                )}
              >
                <div className="flex items-baseline gap-2">
                  <span className="w-16 shrink-0 font-mono text-mono-mini font-semibold uppercase tracking-[0.05em] text-text-dim">
                    {tag(e.tool)}
                  </span>
                  <span className="flex-1 min-w-0 truncate font-mono text-sm font-semibold text-text">
                    {e.summary || e.corpus_id}
                  </span>
                  <span className="shrink-0 font-mono text-xs tabular-nums text-text-dim">
                    {absTime(e.timestamp_ms)} · {relative(now, e.timestamp_ms)}
                  </span>
                </div>
                <div className="flex items-center gap-2 pl-[4.5rem] font-mono text-mono-mini">
                  {e.resolution && (
                    <span className="border border-border-soft px-1 uppercase tracking-[0.05em] text-text-dim rounded-sm">
                      {e.resolution}
                    </span>
                  )}
                  {e.cache_hit && (
                    <span className="border border-border-soft px-1 uppercase tracking-[0.05em] text-success rounded-sm">
                      cache hit
                    </span>
                  )}
                  {e.duration_ms > 0 && (
                    <span className="tabular-nums text-text-dim">
                      {e.duration_ms}ms
                    </span>
                  )}
                  <div className="flex items-center gap-1.5">
                    <span className="h-1.5 w-16 border border-border-soft bg-surface-overlay overflow-hidden">
                      <span
                        className="block h-full bg-accent"
                        style={{
                          width: `${Math.min(100, (delta / maxDelta) * 100)}%`,
                        }}
                      />
                    </span>
                    <span className="tabular-nums text-text-dim">
                      {delta > 0 ? `+${formatTokens(delta)}` : "—"}
                    </span>
                  </div>
                  {e.pressure && PRESSURE_CHIP[e.pressure] && (
                    <span
                      className={cn(
                        "ml-auto uppercase tracking-[0.05em]",
                        PRESSURE_CHIP[e.pressure],
                      )}
                    >
                      {e.pressure}
                    </span>
                  )}
                </div>
              </div>
            );
          })}
        </div>
        {filtered.length > limit && (
          <button
            type="button"
            onClick={() => setLimit((l) => l + PAGE)}
            className="w-full text-left border-t border-border-soft px-3 py-2 font-mono text-mono-mini uppercase tracking-[0.05em] text-text-muted hover:bg-surface-overlay cursor-pointer transition-colors duration-150"
          >
            … {filtered.length - limit} more · load more
          </button>
        )}
      </>
    );
  }

  return (
    <EntitySection chapter={chapter} title="Activity" meta={filtered.length}>
      {events.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5 border-b border-border-soft bg-surface-overlay px-3 py-2">
          <FilterChip
            label="All"
            active={tools.size === 0}
            onClick={() => setTools(new Set())}
          />
          {allTools.map((t) => (
            <FilterChip
              key={t}
              label={t}
              active={tools.has(t)}
              onClick={() => toggleTool(t)}
            />
          ))}
          <div className="flex-1" />
          <FilterChip
            label="cache"
            active={cacheOnly}
            onClick={() => setCacheOnly((v) => !v)}
          />
          <FilterChip
            label="pressure"
            active={pressureOnly}
            onClick={() => setPressureOnly((v) => !v)}
          />
        </div>
      )}
      {body}
    </EntitySection>
  );
}

function FilterChip({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "px-1.5 py-0.5 font-mono text-mono-mini uppercase tracking-[0.05em] border rounded-sm cursor-pointer transition-colors duration-150",
        active
          ? "border-accent bg-accent text-[var(--color-accent-fg-on)]"
          : "border-border-soft text-text-muted hover:border-border",
      )}
    >
      {label}
    </button>
  );
}
