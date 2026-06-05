/**
 * BlastRadiusMap — a symbol's call graph as a PICTURE.
 *
 * ministr's signature question is "if I change this, what breaks?" The
 * SymbolImpact lanes answer it as lists; this is the at-a-glance MAP: a bespoke
 * deterministic SVG where the symbol is a risk-toned core, its transitive
 * CALLERS fan in from above (the blast radius — danger-toned edges curving into
 * the core), and its transitive CALLEES fan out below (accent-toned edges). Edge
 * weight + opacity encode call-graph depth (depth-1 boldest). Every node is
 * keyboard-reachable, aria-labelled, and drills into the stacked inspector;
 * hover/focus lifts a node + its edge and dims the rest.
 *
 * No physics, no dependency — pure layout. Pairs with the detail lanes
 * (SymbolImpact owns the data; this is the additive gestalt).
 */
import { useMemo, useState } from "react";
import type { ImpactedSymbol, SymbolImpact as SymbolImpactData } from "../../lib/types";
import { cn } from "../../lib/utils";

// ── Layout constants (SVG user units; the svg scales to its container). ──────
const W = 400;
const PAD = 16;
const NODE_H = 30;
const NODE_GAP = 9;
const CORE_W = 150;
const CORE_H = 50;
const HALO = 10; // risk-glow ring thickness around the core
const MAX_PER_SIDE = 5; // nodes shown per band before a "+N" marker
const ROW_Y_TOP = PAD + NODE_H / 2;
const CORE_CY = 145;
const ROW_Y_BOT = 2 * CORE_CY - ROW_Y_TOP; // mirror the top band about the core
const HEIGHT = ROW_Y_BOT + NODE_H / 2 + PAD;

type RiskTone = SymbolImpactData["risk"];

/** Risk → the tone class painted on the core ring + halo (non-text). */
const RISK_RING: Record<RiskTone, string> = {
  low: "text-success",
  medium: "text-warning",
  high: "text-danger",
};

interface PlacedNode {
  node: ImpactedSymbol;
  id: string;
  cx: number; // chip centre x
  y: number; // chip top y
  anchorX: number; // where its edge meets the core edge
}

interface OverflowMark {
  count: number;
  cx: number;
  y: number;
}

interface BandLayout {
  placed: PlacedNode[];
  overflow: OverflowMark | null;
  /** Chip width sized to the band's slot count (so chips never overlap). */
  chipW: number;
}

/** Lay a band of nodes out evenly across the inner width, capped with a
 *  trailing "+N" marker; spread their core anchors across the core edge so the
 *  edges don't all converge on a single point. */
function layoutBand(nodes: ImpactedSymbol[], y: number): BandLayout {
  const hasOverflow = nodes.length > MAX_PER_SIDE;
  const shownCount = hasOverflow ? MAX_PER_SIDE - 1 : Math.min(nodes.length, MAX_PER_SIDE);
  const shown = nodes.slice(0, shownCount);
  const slots = shown.length + (hasOverflow ? 1 : 0);
  const innerW = W - 2 * PAD;
  const slotW = slots > 0 ? innerW / slots : innerW;
  const coreL = W / 2 - CORE_W / 2;

  const placed: PlacedNode[] = shown.map((node, i) => {
    const cx = PAD + slotW * (i + 0.5);
    // Anchor spread across the core's top/bottom edge (inset a touch).
    const anchorX = coreL + 14 + ((CORE_W - 28) * (i + 0.5)) / Math.max(slots, 1);
    return { node, id: node.symbol_id || `${node.file}:${node.line}:${i}`, cx, y, anchorX };
  });

  const overflow: OverflowMark | null = hasOverflow
    ? { count: nodes.length - shown.length, cx: PAD + slotW * (slots - 0.5), y }
    : null;

  // Size chips to the slot so they never overlap (a single node stays compact).
  const chipW = Math.min(slotW - NODE_GAP, 104);
  return { placed, overflow, chipW };
}

/** Depth → edge stroke width (depth-1 boldest, fading deeper). */
function edgeWidth(depth: number): number {
  return Math.max(1, 2.8 - (depth - 1) * 0.6);
}
/** Depth → edge opacity when lit. */
function edgeOpacity(depth: number): number {
  return Math.max(0.28, 0.72 - (depth - 1) * 0.16);
}

export interface BlastRadiusMapProps {
  data: SymbolImpactData;
  /** Descend into a caller / callee node (same drill as the lanes). */
  onOpenSymbol: (node: ImpactedSymbol) => void;
}

export function BlastRadiusMap({ data, onOpenSymbol }: BlastRadiusMapProps) {
  const { incoming, outgoing, tests } = useMemo(
    () => ({
      incoming: layoutBand(data.incoming, ROW_Y_TOP - NODE_H / 2),
      outgoing: layoutBand(data.outgoing, ROW_Y_BOT - NODE_H / 2),
      tests: data.tests,
    }),
    [data.incoming, data.outgoing, data.tests],
  );
  const [hover, setHover] = useState<string | null>(null);

  const empty = data.incoming.length === 0 && data.outgoing.length === 0;
  const ring = RISK_RING[data.risk];
  const coreL = W / 2 - CORE_W / 2;
  const coreTop = CORE_CY - CORE_H / 2;
  const coreBot = CORE_CY + CORE_H / 2;
  const covered = tests.length > 0;

  return (
    <svg
      viewBox={`0 0 ${W} ${HEIGHT}`}
      className="w-full"
      style={{ maxHeight: HEIGHT }}
      role="group"
      aria-label={
        empty
          ? "Blast-radius map: a leaf or entry point — nothing calls this symbol and it reaches nothing in the tracked call graph."
          : `Blast-radius map: ${data.incoming_symbols} caller${data.incoming_symbols === 1 ? "" : "s"} (the blast radius), ${data.outgoing_symbols} callee${data.outgoing_symbols === 1 ? "" : "s"}, ${tests.length} covering test${tests.length === 1 ? "" : "s"}. Risk ${data.risk}.`
      }
    >
      {/* Edges first, under the nodes. Butt caps so a thick edge ends flush at
          the core instead of bulging past it. */}
      <g fill="none" strokeLinecap="butt">
        {incoming.placed.map((p) => {
          const lit = !hover || hover === p.id;
          const fromY = p.y + NODE_H;
          const mid = (fromY + coreTop) / 2;
          return (
            <path
              key={`e-in-${p.id}`}
              d={`M${p.cx},${fromY} C${p.cx},${mid} ${p.anchorX},${mid} ${p.anchorX},${coreTop}`}
              stroke="currentColor"
              strokeWidth={edgeWidth(p.node.depth)}
              className="text-danger transition-opacity duration-150 ease-out"
              style={{ opacity: lit ? edgeOpacity(p.node.depth) : 0.06 }}
            />
          );
        })}
        {outgoing.placed.map((p) => {
          const lit = !hover || hover === p.id;
          const mid = (coreBot + p.y) / 2;
          return (
            <path
              key={`e-out-${p.id}`}
              d={`M${p.anchorX},${coreBot} C${p.anchorX},${mid} ${p.cx},${mid} ${p.cx},${p.y}`}
              stroke="currentColor"
              strokeWidth={edgeWidth(p.node.depth)}
              className="text-accent transition-opacity duration-150 ease-out"
              style={{ opacity: lit ? edgeOpacity(p.node.depth) : 0.06 }}
            />
          );
        })}
      </g>

      {/* Caller chips (incoming — the blast radius). */}
      {incoming.placed.map((p) => (
        <NodeChip
          key={`n-in-${p.id}`}
          placed={p}
          chipW={incoming.chipW}
          tone="danger"
          direction="caller"
          dimmed={!!hover && hover !== p.id}
          onHover={setHover}
          onOpen={() => onOpenSymbol(p.node)}
        />
      ))}
      {incoming.overflow && (
        <OverflowChip mark={incoming.overflow} chipW={incoming.chipW} label="more callers" />
      )}

      {/* The symbol core — risk-toned halo + ring, a target glyph, and label. */}
      <g className={ring}>
        <rect
          x={coreL - HALO}
          y={coreTop - HALO}
          width={CORE_W + 2 * HALO}
          height={CORE_H + 2 * HALO}
          rx={16}
          fill="currentColor"
          opacity={empty ? 0.06 : 0.12}
        />
        <rect
          x={coreL}
          y={coreTop}
          width={CORE_W}
          height={CORE_H}
          rx={11}
          className="fill-surface-raised"
          stroke="currentColor"
          strokeWidth={1.5}
        />
        {/* Target glyph — concentric blast rings. */}
        <g
          transform={`translate(${coreL + 25}, ${CORE_CY})`}
          fill="none"
          stroke="currentColor"
        >
          <circle r={10} strokeWidth={1.4} opacity={0.5} />
          <circle r={5.5} strokeWidth={1.6} />
          <circle r={1.6} fill="currentColor" stroke="none" />
        </g>
      </g>
      {/* Core label (high-contrast text, outside the tone group). */}
      <text
        x={coreL + 44}
        y={CORE_CY - 3}
        className="fill-text font-mono"
        style={{ fontSize: 12, fontWeight: 700 }}
      >
        this symbol
      </text>
      <text
        x={coreL + 44}
        y={CORE_CY + 12}
        className="fill-text-dim font-mono"
        style={{ fontSize: 9, letterSpacing: 0.4 }}
      >
        {empty ? "leaf / entry point" : covered ? `${tests.length} test${tests.length === 1 ? "" : "s"}` : "no coverage"}
      </text>

      {/* Callee chips (outgoing). */}
      {outgoing.placed.map((p) => (
        <NodeChip
          key={`n-out-${p.id}`}
          placed={p}
          chipW={outgoing.chipW}
          tone="accent"
          direction="callee"
          dimmed={!!hover && hover !== p.id}
          onHover={setHover}
          onOpen={() => onOpenSymbol(p.node)}
        />
      ))}
      {outgoing.overflow && (
        <OverflowChip mark={outgoing.overflow} chipW={outgoing.chipW} label="more callees" />
      )}
    </svg>
  );
}

function fileTail(path: string): string {
  return path.replace(/\\/g, "/").split("/").slice(-1)[0] ?? path;
}

/** Truncate a name to the text room in a chip of width `w` (full text stays in
 *  the title + aria-label). ~6.1 user-units per mono char at fontSize 10.5. */
function fit(name: string, w: number): string {
  const max = Math.max(3, Math.floor((w - 26) / 6.1));
  return name.length > max ? `${name.slice(0, max - 1)}…` : name;
}

function NodeChip({
  placed,
  chipW,
  tone,
  direction,
  dimmed,
  onHover,
  onOpen,
}: {
  placed: PlacedNode;
  chipW: number;
  tone: "danger" | "accent";
  direction: "caller" | "callee";
  dimmed: boolean;
  onHover: (id: string | null) => void;
  onOpen: () => void;
}) {
  const { node, id, cx, y } = placed;
  const x = cx - chipW / 2;
  const toneStroke = tone === "danger" ? "text-danger" : "text-accent";
  return (
    <g
      role="button"
      tabIndex={0}
      aria-label={`${node.name} — ${direction}, depth ${node.depth} (${node.depth} hop${node.depth === 1 ? "" : "s"}); ${fileTail(node.file)}. Inspect.`}
      className="cursor-pointer"
      style={{ opacity: dimmed ? 0.32 : 1, transition: "opacity 150ms ease-out" }}
      onMouseEnter={() => onHover(id)}
      onMouseLeave={() => onHover(null)}
      onFocus={() => onHover(id)}
      onBlur={() => onHover(null)}
      onClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onOpen();
        }
      }}
    >
      <title>{`${node.name} · ${fileTail(node.file)} · depth ${node.depth}`}</title>
      <rect
        x={x}
        y={y}
        width={chipW}
        height={NODE_H}
        rx={7}
        className={cn("fill-surface", toneStroke)}
        stroke="currentColor"
        strokeWidth={1}
        strokeOpacity={0.55}
      />
      {/* Depth dot — tone on a non-text mark. */}
      <circle cx={x + 10} cy={y + NODE_H / 2} r={3} className={toneStroke} fill="currentColor" />
      <text
        x={x + 19}
        y={y + NODE_H / 2 + 3.5}
        className="fill-text font-mono"
        style={{ fontSize: 10.5 }}
      >
        {fit(node.name, chipW)}
      </text>
    </g>
  );
}

function OverflowChip({ mark, chipW, label }: { mark: OverflowMark; chipW: number; label: string }) {
  const x = mark.cx - chipW / 2;
  return (
    <g aria-label={`and ${mark.count} ${label}`}>
      <rect
        x={x}
        y={mark.y}
        width={chipW}
        height={NODE_H}
        rx={7}
        className="fill-surface-sunken stroke-border-soft"
        strokeWidth={1}
        strokeDasharray="3 3"
      />
      <text
        x={x + chipW / 2}
        y={mark.y + NODE_H / 2 + 3.5}
        textAnchor="middle"
        className="fill-text-dim font-mono tabular-nums"
        style={{ fontSize: 10.5 }}
      >
        +{mark.count}
      </text>
    </g>
  );
}
