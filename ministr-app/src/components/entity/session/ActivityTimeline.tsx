import {
  type ReactNode,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Search, X } from "lucide-react";
import type { ActivityEvent } from "../../../lib/types";
import { formatTokens } from "../../../lib/format";
import {
  formatActivityForDisplay,
  relativizeSummary,
} from "../../../lib/session-activity-summary";
import { relative } from "../../../lib/time";
import { cn } from "../../../lib/utils";
import {
  EntitySection,
  EntitySectionEmpty,
  EntitySectionLoading,
} from "../EntitySection";
import { Chip, ChipGroup } from "../../ui/chip-group";

interface Props {
  chapter: number;
  /** Full session-scoped list, newest first. */
  events: ActivityEvent[];
  loading: boolean;
  error: string | null;
  /** Events newer than this just arrived → flash. */
  flashSince: number;
  /** Substring filter applied to `summary` — set by §1 file-row click-through.
   *  When this changes externally the search field updates to reflect it. */
  targetFilter?: string;
  /** Notify parent when the user clears the search; lets §1 know to
   *  un-highlight any selected file. */
  onTargetFilterChange?: (next: string) => void;
}

const PAGE = 50;

const PRESSURE_CHIP: Record<string, string> = {
  elevated: "text-warning",
  critical: "text-danger",
};

const LATENCY_BAR_PX = 40;

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
 * §Activity — the chronological tool-call timeline. Each event is
 * clickable to expand its full body (no truncation in the expanded
 * panel). Filterable by tool chip + free-text target search; the
 * target search is also driven by §1 file-row clicks via `targetFilter`.
 */
export function ActivityTimeline({
  chapter,
  events,
  loading,
  error,
  flashSince,
  targetFilter,
  onTargetFilterChange,
}: Props) {
  const [tools, setTools] = useState<Set<string>>(new Set());
  const [cacheOnly, setCacheOnly] = useState(false);
  const [pressureOnly, setPressureOnly] = useState(false);
  const [limit, setLimit] = useState(PAGE);
  const [search, setSearch] = useState(targetFilter ?? "");
  const searchInputRef = useRef<HTMLInputElement | null>(null);

  // Mirror an external `targetFilter` prop into the local search box.
  useEffect(() => {
    if (targetFilter !== undefined && targetFilter !== search) {
      setSearch(targetFilter);
    }
    // We intentionally skip `search` in the deps array: the local edit
    // shouldn't fight with the parent prop on every keystroke.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [targetFilter]);

  // Per-tool counts (computed before any filtering so chip counts are
  // stable as the user toggles filters).
  const toolCounts = useMemo(() => {
    const m = new Map<string, number>();
    for (const e of events) {
      const k = tag(e.tool);
      m.set(k, (m.get(k) ?? 0) + 1);
    }
    return m;
  }, [events]);

  const allTools = useMemo(
    () => Array.from(toolCounts.keys()).sort(),
    [toolCounts],
  );

  const searchLower = search.trim().toLowerCase();
  const filtered = useMemo(
    () =>
      events.filter((e) => {
        if (tools.size > 0 && !tools.has(tag(e.tool))) return false;
        if (cacheOnly && !e.cache_hit) return false;
        if (
          pressureOnly &&
          e.pressure !== "elevated" &&
          e.pressure !== "critical"
        ) {
          return false;
        }
        if (
          searchLower &&
          !(e.summary || "").toLowerCase().includes(searchLower)
        ) {
          return false;
        }
        return true;
      }),
    [events, tools, cacheOnly, pressureOnly, searchLower],
  );

  const maxDuration = useMemo(
    () =>
      filtered.reduce((m, e) => Math.max(m, e.duration_ms ?? 0), 0) || 1,
    [filtered],
  );

  const toggleTool = (t: string) =>
    setTools((prev) => {
      const next = new Set(prev);
      if (next.has(t)) next.delete(t);
      else next.add(t);
      return next;
    });

  const updateSearch = (next: string) => {
    setSearch(next);
    onTargetFilterChange?.(next);
  };

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
          className="max-h-96 overflow-y-auto focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
        >
          {visible.map((e) => (
            <EventRow
              key={`${e.timestamp_ms}-${e.tool}-${e.corpus_id}`}
              event={e}
              fresh={e.timestamp_ms > flashSince}
              maxDuration={maxDuration}
              nowMs={now}
            />
          ))}
        </div>
        {filtered.length > limit && (
          <button
            type="button"
            onClick={() => setLimit((l) => l + PAGE)}
            className="w-full text-left border-t border-border-soft px-3 py-2 font-mono text-mono-mini uppercase tracking-[0.08em] text-text-muted hover:bg-surface-overlay cursor-pointer transition-colors duration-150"
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
        <div className="border-b border-border-soft bg-surface-overlay px-3 py-2 space-y-2">
          <ChipGroup>
            <Chip
              label="All"
              count={events.length}
              active={tools.size === 0}
              onClick={() => setTools(new Set())}
            />
            {allTools.map((t) => (
              <Chip
                key={t}
                label={t}
                count={toolCounts.get(t) ?? 0}
                active={tools.has(t)}
                onClick={() => toggleTool(t)}
              />
            ))}
            <span className="flex-1" />
            <Chip
              label="cache"
              active={cacheOnly}
              onClick={() => setCacheOnly((v) => !v)}
            />
            <Chip
              label="pressure"
              active={pressureOnly}
              onClick={() => setPressureOnly((v) => !v)}
            />
          </ChipGroup>
          <label className="flex items-center gap-1.5">
            <Search
              className="h-3.5 w-3.5 shrink-0 text-text-dim"
              strokeWidth={2}
              aria-hidden="true"
            />
            <input
              ref={searchInputRef}
              type="search"
              placeholder="Filter by target (file / symbol / query)…"
              value={search}
              onChange={(ev) => updateSearch(ev.target.value)}
              onKeyDown={(ev) => {
                if (ev.key === "Escape") {
                  ev.preventDefault();
                  updateSearch("");
                  searchInputRef.current?.blur();
                }
              }}
              className="flex-1 min-w-0 bg-transparent font-mono text-xs text-text placeholder:text-text-dim focus:outline-none"
            />
            {search && (
              <button
                type="button"
                onClick={() => updateSearch("")}
                aria-label="Clear filter"
                className="grid h-4 w-4 shrink-0 place-items-center rounded-md text-text-dim hover:text-text cursor-pointer"
              >
                <X className="h-3 w-3" strokeWidth={2} />
              </button>
            )}
          </label>
        </div>
      )}
      {body}
    </EntitySection>
  );
}

function EventRow({
  event,
  fresh,
  maxDuration,
  nowMs,
}: {
  event: ActivityEvent;
  fresh: boolean;
  maxDuration: number;
  nowMs: number;
}) {
  const delta = event.tokens_delta ?? 0;
  const latencyPx = Math.max(
    1,
    Math.min(LATENCY_BAR_PX, (event.duration_ms / maxDuration) * LATENCY_BAR_PX),
  );
  const { head, file, badge } = formatActivityForDisplay(event);
  const displayHead = head || event.corpus_id;
  // Expanded body shows the full summary with absolute paths stripped.
  const expandedTarget = relativizeSummary((event.summary ?? "").trim());

  return (
    <details
      className={cn(
        "group",
        fresh && "ministr-pulse",
      )}
    >
      <summary
        className={cn(
          "grid cursor-pointer list-none items-start px-3 py-1.5 gap-x-3",
          "[grid-template-columns:5.5rem_minmax(0,1fr)_max-content_max-content]",
          "hover:bg-surface-overlay",
          "[&::-webkit-details-marker]:hidden",
        )}
      >
        {/* col 1: TAG — strict 5.5rem column, hard-clipped so long names
            (REFERENCES / DEFINITION) can never bleed into the next cell. */}
        <span className="overflow-hidden whitespace-nowrap font-mono text-[10px] font-semibold uppercase tracking-[0.06em] leading-[18px] text-text-dim">
          {tag(event.tool)}
        </span>

        {/* col 2: TARGET — head on line 1, optional dim file on line 2.
            min-w-0 + overflow-hidden so the cell actually constrains and
            the right columns never get pushed off-screen. */}
        <div className="min-w-0 overflow-hidden leading-[18px]">
          <div className="truncate font-mono text-[12.5px] text-text">
            {displayHead}
          </div>
          {file && (
            <div
              className="truncate font-mono text-[10.5px] text-text-dim mt-px"
              title={file}
            >
              ↳ {file}
            </div>
          )}
        </div>

        {/* col 3: badge — fixed-position chip, aligned with row 1 of the
            target stack so it never collides with the file row below. */}
        <span className="font-mono text-[10px] tabular-nums leading-[18px]">
          {badge ? (
            <span className="whitespace-nowrap rounded border border-border-soft px-1 text-text-dim">
              {badge}
            </span>
          ) : null}
        </span>

        {/* col 4: metrics — latency bar, duration, relative time. */}
        <span className="flex items-center gap-2 whitespace-nowrap font-mono text-[10px] tabular-nums text-text-dim leading-[18px]">
          <span
            className="inline-block h-1.5 border border-border-soft bg-surface-overlay overflow-hidden"
            style={{ width: `${LATENCY_BAR_PX}px` }}
            title={`${event.duration_ms}ms`}
            aria-hidden="true"
          >
            <span
              className="block h-full bg-accent"
              style={{ width: `${latencyPx}px` }}
            />
          </span>
          <span className="w-9 text-right tabular-nums">
            {event.duration_ms > 0 ? `${event.duration_ms}ms` : "—"}
          </span>
          <span className="w-10 text-right">{relative(nowMs, event.timestamp_ms)}</span>
        </span>
      </summary>

      {/* Expanded body */}
      <div className="border-t border-border-soft bg-surface-sunken px-3 py-2.5 space-y-1.5 font-mono text-xs">
        <div className="grid grid-cols-[6rem_1fr] gap-x-3 gap-y-1">
          <span className="text-text-dim uppercase tracking-[0.08em]">Target</span>
          <span className="text-text break-all">
            {expandedTarget || <em className="text-text-dim">—</em>}
          </span>
          <span className="text-text-dim uppercase tracking-[0.08em]">Corpus</span>
          <span className="text-text break-all">{event.corpus_id}</span>
          <span className="text-text-dim uppercase tracking-[0.08em]">When</span>
          <span className="text-text tabular-nums">
            {absTime(event.timestamp_ms)} · {relative(nowMs, event.timestamp_ms)}
          </span>
          <span className="text-text-dim uppercase tracking-[0.08em]">Latency</span>
          <span className="text-text tabular-nums">
            {event.duration_ms > 0 ? `${event.duration_ms}ms` : "—"}
          </span>
          {event.resolution && (
            <>
              <span className="text-text-dim uppercase tracking-[0.08em]">
                Resolution
              </span>
              <span className="text-text">{event.resolution}</span>
            </>
          )}
          {event.cache_hit && (
            <>
              <span className="text-text-dim uppercase tracking-[0.08em]">
                Cache
              </span>
              <span className="text-success">hit</span>
            </>
          )}
          {delta > 0 && (
            <>
              <span className="text-text-dim uppercase tracking-[0.08em]">
                Δ tokens
              </span>
              <span className="text-text tabular-nums">
                +{formatTokens(delta)}
              </span>
            </>
          )}
          {event.pressure && PRESSURE_CHIP[event.pressure] && (
            <>
              <span className="text-text-dim uppercase tracking-[0.08em]">
                Pressure
              </span>
              <span
                className={cn(
                  "uppercase tracking-[0.08em]",
                  PRESSURE_CHIP[event.pressure],
                )}
              >
                {event.pressure}
              </span>
            </>
          )}
        </div>
      </div>
    </details>
  );
}
