/**
 * Session board layout — pure helpers that turn the flat live-session list
 * into the mission-control board: lineage groups (subagents nested under
 * their parent) ordered by pressure (the hottest group floats to the top).
 */
import type { SessionDetail } from "./types";
import { pressureFromUtil, type Pressure } from "./sessions";

const PRESSURE_RANK: Record<Pressure, number> = {
  none: 0,
  low: 1,
  medium: 2,
  high: 3,
  critical: 4,
};

/** Numeric pressure rank (higher = hotter) for sorting. */
export function pressureRank(util: number): number {
  return PRESSURE_RANK[pressureFromUtil(util)];
}

export interface LineageGroup {
  parent: SessionDetail;
  /** Subagent sessions whose parent_session_id is this group's parent. */
  children: SessionDetail[];
  /** Hottest pressure rank across parent + children — drives group order. */
  rank: number;
}

const byPressureDesc = (a: SessionDetail, b: SessionDetail) =>
  pressureRank(b.utilization) - pressureRank(a.utilization) ||
  b.utilization - a.utilization;

/**
 * Group sessions into lineage groups (subagents nested under their parent),
 * each group's children sorted hottest-first, and the groups themselves
 * ordered by their hottest member so a critical session surfaces itself.
 */
export function buildLineageGroups(
  sessions: readonly SessionDetail[],
): LineageGroup[] {
  const ids = new Set(sessions.map((s) => s.session_id));
  const childrenByParent = new Map<string, SessionDetail[]>();
  const roots: SessionDetail[] = [];

  for (const s of sessions) {
    const parent = s.parent_session_id;
    if (parent && ids.has(parent)) {
      const list = childrenByParent.get(parent) ?? [];
      list.push(s);
      childrenByParent.set(parent, list);
    } else {
      roots.push(s);
    }
  }

  const groups: LineageGroup[] = roots.map((parent) => {
    const children = (childrenByParent.get(parent.session_id) ?? [])
      .slice()
      .sort(byPressureDesc);
    const rank = Math.max(
      pressureRank(parent.utilization),
      ...children.map((c) => pressureRank(c.utilization)),
    );
    return { parent, children, rank };
  });

  groups.sort(
    (a, b) => b.rank - a.rank || b.parent.utilization - a.parent.utilization,
  );
  return groups;
}
