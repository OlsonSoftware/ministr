/**
 * SessionLineageTree — the agent SPAWN-FOREST.
 *
 * Mission control's Board nests subagents under their parent as cards; this
 * makes the multi-agent ORCHESTRATION literal: each root agent sits across the
 * top, the subagents it spawned (Claude Code's Task tool → sub-claudes, etc.)
 * hang below it connected by edges — a tidy two-tier forest. Every node is a
 * BUDGET RING (the arc is its context-budget utilization) toned by pressure, so
 * a critical agent reads at a glance; the turn count sits at the centre. Click a
 * node to open its session inspector. A fresh hierarchy idiom for the viz suite,
 * drawn in the shared VizFrame, no backend (reuses the lineage groups + the
 * sessionStatus derivation the Board already computes).
 */
import { useMemo, useState } from "react";
import type { SessionDetail } from "../../lib/types";
import type { LineageGroup } from "../../lib/session-board";
import { sessionStatus } from "../../lib/sessions";
import { toneCssVar } from "../../lib/status";
import { VizFrame } from "../ui/viz-frame";

// ── Layout (SVG user units; the svg scales to its container width). ──────────
const VB_W = 700;
const VB_H = 340;
const PAD = 24;
const ROOT_R = 26;
const CHILD_R = 20;
const COL_GAP = 26;
const TIER_GAP = 120;
const MAX_SCALE = 1; // never enlarge a sparse forest past life size
const SLOT = 2 * ROOT_R + COL_GAP; // column pitch — guarantees no overlap

interface ForestNode {
  key: string;
  s: SessionDetail;
  x: number;
  y: number;
  r: number;
  isRoot: boolean;
}
interface ForestEdge {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
}
interface Forest {
  nodes: ForestNode[];
  edges: ForestEdge[];
  rootCount: number;
  subCount: number;
  /** Uniform layout scale — drives how many label chars fit a column. */
  scale: number;
}

/** Tidy two-tier layout: roots on the top tier (each centred over its
 *  children's column span), subagents on the bottom tier, an empty column
 *  between groups. Built in raw units then uniformly scaled + centred into the
 *  viewBox — the column pitch makes 0-overlap a layout invariant. */
function buildForest(groups: readonly LineageGroup[]): Forest {
  const raw: ForestNode[] = [];
  const rawEdges: ForestEdge[] = [];
  let cursor = 0;
  const rootY = 0;
  const childY = TIER_GAP;

  for (const g of groups) {
    const span = Math.max(1, g.children.length);
    const startCol = cursor;
    const rootX = (startCol + (span - 1) / 2) * SLOT;
    raw.push({ key: g.parent.session_id, s: g.parent, x: rootX, y: rootY, r: ROOT_R, isRoot: true });
    g.children.forEach((c, k) => {
      const cx = (startCol + k) * SLOT;
      raw.push({ key: c.session_id, s: c, x: cx, y: childY, r: CHILD_R, isRoot: false });
      rawEdges.push({ x1: rootX, y1: rootY, x2: cx, y2: childY });
    });
    cursor = startCol + span + 1; // +1 = empty gap column between trees
  }

  if (raw.length === 0) return { nodes: [], edges: [], rootCount: 0, subCount: 0, scale: 1 };

  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const nd of raw) {
    minX = Math.min(minX, nd.x - nd.r);
    maxX = Math.max(maxX, nd.x + nd.r);
    minY = Math.min(minY, nd.y - nd.r);
    maxY = Math.max(maxY, nd.y + nd.r);
  }
  const bw = maxX - minX || 1;
  const bh = maxY - minY || 1;
  const scale = Math.min((VB_W - 2 * PAD) / bw, (VB_H - 2 * PAD) / bh, MAX_SCALE);
  const cx = (minX + maxX) / 2;
  const cy = (minY + maxY) / 2;
  const tx = VB_W / 2 - cx * scale;
  const ty = VB_H / 2 - cy * scale;

  const nodes = raw.map((nd) => ({ ...nd, x: nd.x * scale + tx, y: nd.y * scale + ty, r: nd.r * scale }));
  const edges = rawEdges.map((e) => ({
    x1: e.x1 * scale + tx,
    y1: e.y1 * scale + ty,
    x2: e.x2 * scale + tx,
    y2: e.y2 * scale + ty,
  }));
  const subCount = raw.filter((n) => !n.isRoot).length;
  return { nodes, edges, rootCount: raw.length - subCount, subCount, scale };
}

/** Short, legible agent label — the MCP client name, else the id tail. */
function agentLabel(s: SessionDetail): string {
  if (s.client_name) {
    return s.client_name.length > 18 ? `${s.client_name.slice(0, 17)}…` : s.client_name;
  }
  return `…${s.session_id.slice(-4)}`;
}

/** Truncate a label to the column's pixel room (~6px per mono char at 10px),
 *  so adjacent subagent labels never collide regardless of fan-out width. */
function fitLabel(label: string, maxChars: number): string {
  return label.length > maxChars ? `${label.slice(0, Math.max(1, maxChars - 1))}…` : label;
}

function clampFont(r: number): number {
  return Math.max(9, Math.min(14, Math.round(r * 0.78)));
}

export interface SessionLineageTreeProps {
  groups: readonly LineageGroup[];
  /** Open a session's inspector (zoom into the agent). */
  onOpen: (s: SessionDetail) => void;
  className?: string;
}

export function SessionLineageTree({ groups, onOpen, className }: SessionLineageTreeProps) {
  const forest = useMemo(() => buildForest(groups), [groups]);
  const [hover, setHover] = useState<string | null>(null);

  if (forest.nodes.length === 0) return null;

  const hovered = hover ? forest.nodes.find((n) => n.key === hover) : null;
  // Column pitch in scaled px → how many 10px-mono chars a label may use.
  const labelMax = Math.max(4, Math.floor((SLOT * forest.scale * 0.9) / 6));

  return (
    <VizFrame
      className={className}
      readout={
        <>
          <span>
            <span className="tabular-nums font-semibold text-text">{forest.rootCount}</span>{" "}
            {forest.rootCount === 1 ? "agent" : "agents"}
            {forest.subCount > 0 && (
              <>
                {" · "}
                <span className="tabular-nums font-semibold text-text">{forest.subCount}</span>{" "}
                {forest.subCount === 1 ? "subagent" : "subagents"}
              </>
            )}
          </span>
          <span>ring = budget used · click to inspect</span>
        </>
      }
    >
      <div className="-mx-1">
        <svg
          viewBox={`0 0 ${VB_W} ${VB_H}`}
          className="w-full"
          style={{ maxHeight: VB_H }}
          role="group"
          aria-label={`Agent spawn forest: ${forest.rootCount} root agent${forest.rootCount === 1 ? "" : "s"} and ${forest.subCount} subagent${forest.subCount === 1 ? "" : "s"}; each node's ring is its context-budget utilization.`}
        >
          {/* Spawn edges (behind the nodes) — neutral so node tones carry meaning. */}
          {forest.edges.map((e, i) => (
            <line
              key={i}
              x1={e.x1}
              y1={e.y1}
              x2={e.x2}
              y2={e.y2}
              style={{ stroke: "var(--color-border-hover)" }}
              strokeWidth={1.5}
              strokeOpacity={0.8}
            />
          ))}

          {forest.nodes.map((nd) => {
            const st = sessionStatus(nd.s);
            const lit = !hover || hover === nd.key;
            const sw = Math.max(2.5, nd.r * 0.16);
            const live = st.pressure === "high" || st.pressure === "critical";
            const fs = clampFont(nd.r);
            return (
              <g
                key={nd.key}
                role="button"
                tabIndex={0}
                aria-label={`${agentLabel(nd.s)} — turn ${nd.s.current_turn}, ${st.pct}% budget used, ${st.pressure} pressure${nd.isRoot ? "" : ", subagent"}. Open session inspector.`}
                style={{
                  color: toneCssVar(st.tone),
                  opacity: lit ? 1 : 0.4,
                  transition: "opacity 150ms ease-out",
                  cursor: "pointer",
                }}
                onMouseEnter={() => setHover(nd.key)}
                onMouseLeave={() => setHover(null)}
                onFocus={() => setHover(nd.key)}
                onBlur={() => setHover(null)}
                onClick={() => onOpen(nd.s)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    onOpen(nd.s);
                  }
                }}
              >
                <title>{`${agentLabel(nd.s)} · turn ${nd.s.current_turn} · ${st.pct}% budget · ${st.pressure}`}</title>
                {/* Pressure halo — opacity-pulse only (reduced-motion-safe). */}
                {live && (
                  <circle
                    cx={nd.x}
                    cy={nd.y}
                    r={nd.r + 5}
                    fill="none"
                    stroke="currentColor"
                    strokeOpacity={0.5}
                    strokeWidth={1.5}
                    className="motion-safe:animate-pulse"
                  />
                )}
                {/* Budget ring — track + utilization arc (starts at top). */}
                <circle
                  cx={nd.x}
                  cy={nd.y}
                  r={nd.r}
                  style={{ fill: "var(--color-surface)" }}
                  stroke="currentColor"
                  strokeOpacity={nd.isRoot ? 0.22 : 0.18}
                  strokeWidth={sw}
                />
                {st.pct > 0 && (
                  <circle
                    cx={nd.x}
                    cy={nd.y}
                    r={nd.r}
                    fill="none"
                    stroke="currentColor"
                    strokeWidth={sw}
                    strokeLinecap="round"
                    pathLength={100}
                    strokeDasharray={`${st.pct} 100`}
                    transform={`rotate(-90 ${nd.x} ${nd.y})`}
                  />
                )}
                {/* Turn count at the centre. */}
                <text
                  x={nd.x}
                  y={nd.y}
                  textAnchor="middle"
                  dominantBaseline="central"
                  className="fill-text font-mono font-semibold tabular-nums"
                  style={{ fontSize: fs }}
                >
                  {nd.s.current_turn}
                </text>
                {/* Agent label below — only when the node has room (else hover/aria). */}
                {nd.r >= 16 && (
                  <text
                    x={nd.x}
                    y={nd.y + nd.r + 12}
                    textAnchor="middle"
                    className="fill-text-dim font-mono"
                    style={{ fontSize: 10 }}
                  >
                    {fitLabel(agentLabel(nd.s), labelMax)}
                  </text>
                )}
              </g>
            );
          })}

          {/* Hover caption — bottom-anchored so it never occludes the forest. */}
          {hovered && (
            <text
              x={VB_W / 2}
              y={VB_H - 6}
              textAnchor="middle"
              className="fill-text font-mono"
              style={{ fontSize: 11 }}
            >
              {`${agentLabel(hovered.s)} · turn ${hovered.s.current_turn} · ${sessionStatus(hovered.s).pct}% budget · ${sessionStatus(hovered.s).pressure}${hovered.isRoot ? "" : " · subagent"}`}
            </text>
          )}
        </svg>
      </div>
    </VizFrame>
  );
}
