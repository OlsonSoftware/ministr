/**
 * SessionsSurface — mission control: a live board of every agent session
 * consuming the cache. Cards EXPAND IN PLACE to a per-session economics
 * dashboard (no panel/route switch); subagents NEST under their parent as a
 * lineage tree; the board AUTO-SORTS by pressure so a critical session
 * surfaces itself. The deep EntityPanel inspector remains one click away from
 * the expanded view. Reuses the shared `useSessions` store + `lib/sessions`
 * derivations — no own fetch.
 */
import { useMemo, useState } from "react";
import { Activity, GitFork, LayoutGrid, Users } from "@/components/ui/icons";
import { motion } from "motion/react";

import type { CorpusInfo, DaemonStatus, SessionDetail } from "../../lib/types";
import { formatTokens } from "../../lib/format";
import { toneTextClass } from "../../lib/status";
import {
  clampPct,
  utilizationTone,
  type SessionSample,
} from "../../lib/sessions";
import { buildLineageGroups, type LineageGroup } from "../../lib/session-board";
import { listContainer, listItem } from "../../lib/motion";
import { useEntityPanel } from "../../hooks/useEntityPanel";
import { useSessions } from "../../hooks/useSessions";
import { useSessionActivity } from "../../hooks/useSessionActivity";

import { EmptyState } from "../ui/empty-state";
import { NumberTicker } from "../ui/number-ticker";
import { SessionCard, SessionCardSkeleton } from "../ui/session-card";
import { FacetHeader } from "../ui/facet-header";
import { AdaptiveSurface } from "../ui/adaptive-surface";
import { ViewSwitch, type ViewOption } from "../ui/view-switch";
import { ActivityPulseConnector } from "./ActivityPulse";
import { SessionLineageTree } from "./SessionLineageTree";

const CONNECT_CMD = "npx @modelcontextprotocol/inspector ministr stdio";

type ActivityView = "board" | "tree";

const ACTIVITY_VIEWS: ViewOption<ActivityView>[] = [
  { id: "board", label: "Board", icon: LayoutGrid, hint: "Live cards — expand for per-agent economics" },
  { id: "tree", label: "Tree", icon: GitFork, hint: "Spawn forest — agents & subagents by budget" },
];

export function SessionsSurface({
  status,
  activeCorpusId,
  initialView,
}: {
  status: DaemonStatus;
  activeCorpusId: string | null;
  /** Initial view mode — Storybook renders the spawn-forest directly with "tree". */
  initialView?: ActivityView;
}) {
  const { sessions, byId, samples, freshIds, loaded } = useSessions();
  const { openEntity } = useEntityPanel();
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [view, setView] = useState<ActivityView>(initialView ?? "board");

  // Activity is a FACET of the spine: when a project is selected it shows only
  // that project's agents; on the Fleet (activeCorpusId === null) it shows the
  // whole fleet. (OOUX: one context — the facet never re-picks a corpus.)
  const scoped = useMemo(
    () =>
      activeCorpusId
        ? sessions.filter((s) => s.corpus_id === activeCorpusId)
        : sessions,
    [sessions, activeCorpusId],
  );

  const corpora = status.corpora;
  const corpusById = useMemo(
    () => new Map(corpora.map((c) => [c.id, c])),
    [corpora],
  );

  const agg = useMemo(() => {
    const used = scoped.reduce((a, s) => a + s.tokens_used, 0);
    const cap = scoped.reduce(
      (a, s) => a + s.tokens_used + s.tokens_remaining,
      0,
    );
    return {
      count: scoped.length,
      util: cap > 0 ? used / cap : 0,
      saved: scoped.reduce((a, s) => a + s.total_tokens_saved, 0),
      dedup: scoped.reduce((a, s) => a + s.dedup_hits, 0),
    };
  }, [scoped]);

  // Pressure-sorted lineage groups (subagents nested under their parent).
  const groups = useMemo(() => buildLineageGroups(scoped), [scoped]);

  const toggle = (id: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const openInspector = (s: SessionDetail) =>
    openEntity({
      kind: "session",
      corpusId: s.corpus_id,
      sessionId: s.session_id,
      seed: byId.get(s.session_id) ?? s,
    });

  const cardProps = (s: SessionDetail) => ({
    session: s,
    corpus: corpusById.get(s.corpus_id),
    samples: samples.get(s.session_id) ?? [],
    fresh: freshIds.has(s.session_id),
    expanded: expanded.has(s.session_id),
    onToggle: () => toggle(s.session_id),
    onOpenInspector: () => openInspector(s),
  });

  return (
    <AdaptiveSurface>
      <div className="h-full flex flex-col min-h-0">
        <FacetHeader
          icon={Activity}
          title="Activity"
          glance={
            agg.count === 0
              ? "No agents connected."
              : `${agg.count} live agent ${agg.count === 1 ? "session" : "sessions"}.`
          }
          actions={
            agg.count > 0 ? (
              <div className="flex items-center gap-4">
                <ViewSwitch
                  value={view}
                  onChange={setView}
                  options={ACTIVITY_VIEWS}
                  ariaLabel="Activity view"
                />
                <div className="flex items-center gap-5">
                  <AggStat label="budget">
                    <span className={toneTextClass(utilizationTone(agg.util))}>
                      {clampPct(agg.util * 100)}%
                    </span>
                  </AggStat>
                  <AggStat label="saved">
                    <NumberTicker value={agg.saved} format={formatTokens} />
                  </AggStat>
                  <AggStat label="dedup">
                    <NumberTicker value={agg.dedup} flashOnChange />
                  </AggStat>
                </div>
              </div>
            ) : undefined
          }
        />

        <div className="flex-1 min-h-0 overflow-y-auto px-5 pb-5">
          {!loaded ? (
            <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3 items-start">
              {Array.from({ length: 6 }).map((_, i) => (
                <SessionCardSkeleton key={i} />
              ))}
            </div>
          ) : scoped.length === 0 ? (
            <div className="grid place-items-center h-full">
              <EmptyState
                icon={Users}
                accent
                title="No active sessions"
                hint={
                  <span className="block">
                    Point Claude Code, Cursor, or any MCP client at a project
                    and its agents appear here live.
                    <button
                      type="button"
                      onClick={() => navigator.clipboard.writeText(CONNECT_CMD)}
                      title="Click to copy"
                      className="mt-3 block w-full text-left rounded-md border border-border bg-surface-sunken px-3 py-2 font-mono text-mono-mini text-text break-all cursor-pointer hover:border-border-hover transition-colors duration-150"
                    >
                      {`$ ${CONNECT_CMD}`}
                    </button>
                  </span>
                }
              />
            </div>
          ) : (
            <>
              {/* The live tool-call heartbeat — the WHEN gestalt, above both
                  views. Polls only while there are live sessions (here). */}
              <ActivityPulseConnector
                corpusId={activeCorpusId}
                className="mb-3"
              />
              {view === "tree" ? (
                /* The WHO/lineage gestalt — the agent spawn-forest. */
                <SessionLineageTree groups={groups} onOpen={openInspector} />
              ) : (
                <motion.div
                  variants={listContainer}
                  initial="initial"
                  animate="animate"
                  className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3 items-start"
                >
                  {groups.map((group) => (
                    <LineageGroupCell
                      key={group.parent.session_id}
                      group={group}
                      cardProps={cardProps}
                    />
                  ))}
                </motion.div>
              )}
            </>
          )}
        </div>
      </div>
    </AdaptiveSurface>
  );
}

function AggStat({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col items-end">
      <span className="font-mono text-base font-semibold tabular-nums text-text">
        {children}
      </span>
      <span className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
        {label}
      </span>
    </div>
  );
}

type CardProps = {
  session: SessionDetail;
  corpus: CorpusInfo | undefined;
  samples: readonly SessionSample[];
  fresh: boolean;
  expanded: boolean;
  onToggle: () => void;
  onOpenInspector: () => void;
};

/** A board SessionCard backed by a live per-session activity feed. The feed
 *  fetches (useSessionActivity polls recent_activity) ONLY while the card is
 *  expanded — a collapsed card passes `null`, so the resting board makes no
 *  per-session requests. The expanded card then shows a recent-activity peek +
 *  literal last-activity (aaa-sessions-live-detail). */
function BoardSessionCard({
  props,
  child = false,
}: {
  props: CardProps;
  child?: boolean;
}) {
  const { events, loading } = useSessionActivity(
    props.expanded ? props.session.session_id : null,
  );
  return (
    <SessionCard
      interaction="expand"
      {...props}
      child={child}
      activity={props.expanded ? events : undefined}
      activityLoading={props.expanded ? loading : false}
    />
  );
}

/** One lineage group: the parent card plus its subagents nested beneath. */
function LineageGroupCell({
  group,
  cardProps,
}: {
  group: LineageGroup;
  cardProps: (s: SessionDetail) => CardProps;
}) {
  return (
    <motion.div variants={listItem} layout className="flex flex-col gap-2">
      <BoardSessionCard props={cardProps(group.parent)} />
      {group.children.length > 0 && (
        <div className="ml-3 pl-3 border-l border-border-soft flex flex-col gap-2">
          <span className="pl-0.5 font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
            {group.children.length} subagent
            {group.children.length === 1 ? "" : "s"}
          </span>
          {group.children.map((c) => (
            <BoardSessionCard key={c.session_id} props={cardProps(c)} child />
          ))}
        </div>
      )}
    </motion.div>
  );
}
