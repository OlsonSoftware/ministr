// F2.5 — bridge-graph hero (screenshottable differentiator).
//
// Static SVG depicting the cross-language bridge story: a small
// constellation of nodes (Rust, TypeScript, Python) connected by
// labelled edges (`tauri_command`, `pyo3`, `napi`). Renders zero
// client JS so Lighthouse stays ≥ 95 on the marketing pages.
//
// SOLID seam for F3.6: the data lives in [`BRIDGE_GRAPH_SAMPLE`] as
// a typed structure. F3.6's web visualizer (Team feature) imports
// the same [`Node`] / [`Edge`] shapes and renders a live corpus.
// `BridgeGraphHero` here is the marketing case where the data is
// fixed; the live view becomes a wrapping component that swaps the
// data prop.

import type { ReactNode } from 'react';

/** One symbol or file in the bridge graph. */
export interface Node {
  id: string;
  /** Display label. */
  label: string;
  /** Language slug — drives the node colour. */
  lang: 'rust' | 'typescript' | 'python';
  /** Polar coordinates around the centre, in degrees + pixels. */
  angle: number;
  radius: number;
}

/** One cross-language bridge edge. */
export interface Edge {
  from: string;
  to: string;
  /** Bridge kind — one of the 12 detectors ministr-core ships. */
  kind: 'tauri_command' | 'pyo3' | 'napi';
}

/**
 * F3.6-b — wider node type for the live interactive visualizer. The
 * marketing hero's narrow `Node` union (3 languages, 3 kinds) lands
 * structurally inside this type, so `BRIDGE_GRAPH_SAMPLE` can be
 * passed to either consumer without conversion. Live data from the
 * F3.6-a backend endpoint speaks the wider shape (~20 languages,
 * 12 bridge kinds).
 *
 * `angle`/`radius` are optional — live data won't carry polar coords;
 * the interactive component computes a circular auto-layout in that
 * case.
 */
export interface LiveBridgeNode {
  id: string;
  label: string;
  /** Language slug (any string — colours fall back for unknowns). */
  lang: string;
  /** Optional polar layout hint. */
  angle?: number;
  radius?: number;
  /**
   * Source file the symbol lives in. The F3.6-a backend wire shape
   * always carries this; the F2.5 marketing sample omits it. F3.6-c-i
   * uses it for the file substring filter.
   */
  file?: string;
}

/** F3.6-b — wider edge type for the live interactive visualizer. */
export interface LiveBridgeEdge {
  from: string;
  to: string;
  /** Bridge mechanism kind (any of the 12 detectors). */
  kind: string;
}

/** Marketing-hero sample. Mirrors the Tauri-heavy ministr-app pattern
 *  so a real screenshot of the F3.6 web visualizer can plausibly
 *  match this shape. */
export const BRIDGE_GRAPH_SAMPLE: { nodes: Node[]; edges: Edge[] } = {
  nodes: [
    { id: 'tauri_invoke', label: 'invoke()', lang: 'typescript', angle: -90, radius: 160 },
    { id: 'cmd_cloud_status', label: 'cloud_status', lang: 'rust', angle: 30, radius: 160 },
    { id: 'cmd_cloud_clone', label: 'cloud_clone_repo', lang: 'rust', angle: 150, radius: 160 },
    { id: 'py_handler', label: 'handle_event', lang: 'python', angle: 90, radius: 230 },
    { id: 'napi_indexer', label: 'index_corpus', lang: 'typescript', angle: 210, radius: 230 },
  ],
  edges: [
    { from: 'tauri_invoke', to: 'cmd_cloud_status', kind: 'tauri_command' },
    { from: 'tauri_invoke', to: 'cmd_cloud_clone', kind: 'tauri_command' },
    { from: 'cmd_cloud_clone', to: 'py_handler', kind: 'pyo3' },
    { from: 'cmd_cloud_clone', to: 'napi_indexer', kind: 'napi' },
  ],
};

const LANG_COLOR: Record<Node['lang'], string> = {
  rust: '#dea584',
  typescript: '#3178c6',
  python: '#3776ab',
};

const KIND_COLOR: Record<Edge['kind'], string> = {
  tauri_command: '#facc15',
  pyo3: '#a78bfa',
  napi: '#4ade80',
};

interface BridgeGraphHeroProps {
  /** Override the sample data — F3.6 will pass live corpus data. */
  data?: { nodes: Node[]; edges: Edge[] };
  /** Slot for a caption rendered below the graph. */
  caption?: ReactNode;
  className?: string;
}

/**
 * Static SVG depicting cross-language bridge edges. Renders inline so
 * the marketing pages can embed it without an image dep + accessor
 * round-trip.
 */
export function BridgeGraphHero({
  data = BRIDGE_GRAPH_SAMPLE,
  caption,
  className,
}: BridgeGraphHeroProps) {
  const size = 560;
  const cx = size / 2;
  const cy = size / 2;
  const byId = new Map(data.nodes.map((n) => [n.id, n]));

  function pos(node: Node): { x: number; y: number } {
    const rad = (node.angle * Math.PI) / 180;
    return { x: cx + Math.cos(rad) * node.radius, y: cy + Math.sin(rad) * node.radius };
  }

  return (
    <figure className={className}>
      <svg
        viewBox={`0 0 ${size} ${size}`}
        role="img"
        aria-label="Bridge graph: invoke() calls Rust commands that themselves call into Python and TypeScript across language boundaries."
        className="h-auto w-full"
      >
        <defs>
          <marker
            id="bridge-arrow"
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="6"
            markerHeight="6"
            orient="auto-start-reverse"
          >
            <path d="M0,0 L10,5 L0,10 z" fill="currentColor" opacity="0.6" />
          </marker>
        </defs>

        {/* Edges first so node circles draw on top. */}
        {data.edges.map((e) => {
          const a = byId.get(e.from);
          const b = byId.get(e.to);
          if (!a || !b) return null;
          const pa = pos(a);
          const pb = pos(b);
          const mx = (pa.x + pb.x) / 2;
          const my = (pa.y + pb.y) / 2;
          return (
            <g key={`${e.from}-${e.to}`}>
              <line
                x1={pa.x}
                y1={pa.y}
                x2={pb.x}
                y2={pb.y}
                stroke={KIND_COLOR[e.kind]}
                strokeWidth="2"
                opacity="0.55"
                markerEnd="url(#bridge-arrow)"
              />
              <text
                x={mx}
                y={my - 6}
                textAnchor="middle"
                className="text-[11px]"
                fill="currentColor"
                opacity="0.7"
                fontFamily="ui-monospace, SFMono-Regular, monospace"
              >
                {e.kind}
              </text>
            </g>
          );
        })}

        {/* Nodes. */}
        {data.nodes.map((n) => {
          const p = pos(n);
          return (
            <g key={n.id}>
              <circle
                cx={p.x}
                cy={p.y}
                r="22"
                fill={LANG_COLOR[n.lang]}
                opacity="0.85"
                stroke="currentColor"
                strokeOpacity="0.4"
                strokeWidth="1"
              />
              <text
                x={p.x}
                y={p.y + 38}
                textAnchor="middle"
                className="text-[12px]"
                fill="currentColor"
                fontFamily="ui-monospace, SFMono-Regular, monospace"
              >
                {n.label}
              </text>
              <text
                x={p.x}
                y={p.y + 4}
                textAnchor="middle"
                className="text-[10px]"
                fill="#0a0a0a"
                fontFamily="ui-monospace, SFMono-Regular, monospace"
                fontWeight="600"
              >
                {n.lang.slice(0, 2).toUpperCase()}
              </text>
            </g>
          );
        })}
      </svg>
      {caption ? (
        <figcaption className="mt-3 text-center text-xs text-fd-muted-foreground">
          {caption}
        </figcaption>
      ) : null}
    </figure>
  );
}
