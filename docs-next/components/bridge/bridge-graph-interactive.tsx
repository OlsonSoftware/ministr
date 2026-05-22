// F3.6-b-i — interactive bridge-graph visualizer.
//
// Wraps `@xyflow/react`'s `<ReactFlow>` with the wider
// `LiveBridgeNode`/`LiveBridgeEdge` shapes (defined in
// `../landing/bridge-graph.tsx`). The narrow marketing-hero
// `Node`/`Edge` types are structural subtypes, so this component
// also accepts `BRIDGE_GRAPH_SAMPLE` unchanged.
//
// # Why a separate component
//
// F2.5's `BridgeGraphHero` is a static SVG with zero client JS —
// part of the marketing /pricing page's Lighthouse ≥ 95 contract.
// This component adds DOM-only react-flow surface (`"use client"`),
// so it lives separately and is only mounted on Team-tier pages
// (demo for now, /orgs/.../bridge once F3.6-b-iii lands docs-next
// auth).
//
// # Layout
//
// Nodes that carry an `angle`/`radius` polar hint (from the marketing
// sample) are positioned at exactly that point so a side-by-side
// screenshot of the F2.5 hero and this component matches. Live
// nodes that don't carry polar coords get a deterministic circular
// auto-layout by id-hash → angle, so the same input always produces
// the same picture (matters for the F3.6-d PNG export step).

'use client';

import { ReactFlow, Background, Controls, type Edge as RfEdge, type Node as RfNode } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import type { CSSProperties } from 'react';

import type { LiveBridgeEdge, LiveBridgeNode } from '../landing/bridge-graph';
import { BridgeGraphExport } from './bridge-graph-export';

interface BridgeGraphInteractiveProps {
  data: { nodes: ReadonlyArray<LiveBridgeNode>; edges: ReadonlyArray<LiveBridgeEdge> };
  /** Square canvas pixel size; defaults to 560 to match the F2.5 hero. */
  size?: number;
  className?: string;
  /** Optional caption rendered below the canvas. */
  caption?: React.ReactNode;
  /**
   * F3.6-c-ii-a — fired with the original `LiveBridgeEdge` when an
   * edge is clicked. The parent wrapper uses this to drive a side
   * panel with the export/import source pair. Optional; when
   * absent, edge clicks are no-ops.
   */
  onEdgeClick?: (edge: LiveBridgeEdge) => void;
}

// Language-slug → fill colour. Mirrors the F2.5 narrow palette where
// it overlaps; unknowns fall back to a neutral slate so the canvas
// never renders an undefined fill.
const LANG_COLOR: Record<string, string> = {
  rust: '#dea584',
  typescript: '#3178c6',
  javascript: '#f7df1e',
  python: '#3776ab',
  go: '#00add8',
  c: '#a8b9cc',
  cpp: '#00599c',
  csharp: '#9b4f96',
  java: '#b07219',
  kotlin: '#a97bff',
  swift: '#f05138',
  ruby: '#cc342d',
  php: '#777bb4',
  dart: '#00b4ab',
  zig: '#f7a41d',
};
const LANG_FALLBACK = '#64748b'; // slate-500

// Bridge-kind → stroke colour. Mirrors the F2.5 palette + extends.
const KIND_COLOR: Record<string, string> = {
  tauri_command: '#facc15',
  tauri_event: '#fde047',
  pyo3: '#a78bfa',
  napi: '#4ade80',
  wasm_bindgen: '#f97316',
  uniffi: '#fb7185',
  jni: '#06b6d4',
  cgo: '#22d3ee',
  ffi: '#94a3b8',
  grpc: '#0ea5e9',
  http_route: '#10b981',
  flutter: '#3b82f6',
  electron: '#7dd3fc',
};
const KIND_FALLBACK = '#94a3b8'; // slate-400

/**
 * Deterministic hash → angle for nodes that lack polar coords. Cheap
 * djb2-style hash keeps the auto-layout stable across renders, which
 * matters for the F3.6-d export step: identical input must produce
 * identical PNG output.
 */
function hashToAngle(id: string): number {
  let h = 5381;
  for (let i = 0; i < id.length; i += 1) {
    h = (h * 33) ^ id.charCodeAt(i);
  }
  // map h to [0, 360)
  return (((h % 360) + 360) % 360);
}

function nodePosition(
  node: LiveBridgeNode,
  fallbackRadius: number,
  center: number,
): { x: number; y: number } {
  const angle = node.angle ?? hashToAngle(node.id);
  const radius = node.radius ?? fallbackRadius;
  const rad = (angle * Math.PI) / 180;
  return { x: center + Math.cos(rad) * radius, y: center + Math.sin(rad) * radius };
}

export function BridgeGraphInteractive({
  data,
  size = 560,
  className,
  caption,
  onEdgeClick,
}: BridgeGraphInteractiveProps) {
  const center = size / 2;
  const fallbackRadius = Math.min(size * 0.34, 200);

  const rfNodes: RfNode[] = data.nodes.map((n) => {
    const { x, y } = nodePosition(n, fallbackRadius, center);
    const color = LANG_COLOR[n.lang] ?? LANG_FALLBACK;
    const style: CSSProperties = {
      background: color,
      color: '#0f172a',
      border: '2px solid rgba(15,23,42,0.55)',
      borderRadius: 999,
      padding: '6px 12px',
      fontFamily: 'ui-monospace, SFMono-Regular, monospace',
      fontSize: 12,
      boxShadow: '0 1px 2px rgba(0,0,0,0.25)',
    };
    return {
      id: n.id,
      data: { label: n.label, lang: n.lang },
      position: { x, y },
      style,
      // Inputs/outputs auto-attach on either side; xyflow handles routing.
      sourcePosition: undefined,
      targetPosition: undefined,
    };
  });

  const rfEdges: RfEdge[] = data.edges.map((e) => {
    const stroke = KIND_COLOR[e.kind] ?? KIND_FALLBACK;
    return {
      id: `${e.from}->${e.to}-${e.kind}`,
      source: e.from,
      target: e.to,
      label: e.kind,
      labelStyle: { fontFamily: 'ui-monospace, SFMono-Regular, monospace', fontSize: 11 },
      labelBgPadding: [4, 2],
      labelBgBorderRadius: 4,
      labelBgStyle: { fill: 'rgba(15,23,42,0.55)' },
      style: { stroke, strokeWidth: 2, opacity: 0.85 },
      animated: false,
      // F3.6-c-ii-a — stash the original LiveBridgeEdge so onEdgeClick
      // can hand it back to the parent without lossy id parsing.
      data: { liveEdge: e },
    };
  });

  return (
    <figure className={className}>
      <div
        style={{ width: '100%', height: size }}
        aria-label="Interactive bridge graph — zoom, pan, hover for cross-language link details."
        role="img"
      >
        <ReactFlow
          nodes={rfNodes}
          edges={rfEdges}
          fitView
          fitViewOptions={{ padding: 0.15 }}
          nodesDraggable={false}
          nodesConnectable={false}
          elementsSelectable
          onEdgeClick={
            onEdgeClick
              ? (_e, edge) => {
                  const live = (edge.data as { liveEdge?: LiveBridgeEdge } | undefined)?.liveEdge;
                  if (live) onEdgeClick(live);
                }
              : undefined
          }
          proOptions={{ hideAttribution: true }}
        >
          <Background gap={24} size={1} />
          <Controls showInteractive={false} />
          <BridgeGraphExport />
        </ReactFlow>
      </div>
      {caption ? (
        <figcaption className="mt-2 text-center text-sm text-slate-500">{caption}</figcaption>
      ) : null}
    </figure>
  );
}
