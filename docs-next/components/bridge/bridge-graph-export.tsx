// F3.6-d — PNG / SVG export buttons for the bridge visualizer.
//
// Implements the official react-flow approach (xyflow team's own
// download-image example at reactflow.dev/examples/misc/download-image):
//   1. `useReactFlow().getNodes()` — current rendered nodes (already
//      filtered by F3.6-c-i).
//   2. `getNodesBounds(nodes)` — bounding box of the visible subset.
//   3. `getViewportForBounds(...)` — compute the transform that fits
//      that box into the target image dimensions with padding.
//   4. `toPng` / `toSvg` from `html-to-image` against the
//      `.react-flow__viewport` DOM element, overriding the live CSS
//      transform with the computed one so offscreen edges are NOT
//      clipped (the well-known issue github.com/xyflow/xyflow/2118).
//
// Both PNG (white background, design-doc friendly) and SVG (transparent
// background, composable in vector pipelines) are offered. Both
// honour the current F3.6-c-i filter state because we render whatever
// `getNodes()` returns — already filtered by the upstream wrapper.
//
// Mount as a child of `<ReactFlow>` (via `<Panel>`) so `useReactFlow()`
// returns the active context.

'use client';

import { Panel, getNodesBounds, getViewportForBounds, useReactFlow } from '@xyflow/react';
import { toPng, toSvg } from 'html-to-image';
import { useState } from 'react';

interface BridgeGraphExportProps {
  /** Output image dimensions. Larger = sharper PNG, no effect on SVG. */
  imageWidth?: number;
  imageHeight?: number;
  /** Base filename (no extension). Default `bridge-graph`. */
  filename?: string;
}

/** Anchor-element + dataURL download trick — the standard pattern. */
function downloadDataUrl(dataUrl: string, filename: string) {
  const a = document.createElement('a');
  a.setAttribute('download', filename);
  a.setAttribute('href', dataUrl);
  a.click();
}

export function BridgeGraphExport({
  imageWidth = 1600,
  imageHeight = 1200,
  filename = 'bridge-graph',
}: BridgeGraphExportProps) {
  const { getNodes } = useReactFlow();
  const [busy, setBusy] = useState<'png' | 'svg' | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function exportImage(format: 'png' | 'svg') {
    setError(null);
    setBusy(format);
    try {
      const nodes = getNodes();
      if (nodes.length === 0) {
        throw new Error('no nodes to export');
      }
      const bounds = getNodesBounds(nodes);
      // padding=0.5: 50% of the bounding-box dim around all sides.
      // minZoom=0.2, maxZoom=2: keep the export legible.
      const viewport = getViewportForBounds(bounds, imageWidth, imageHeight, 0.2, 2, 0);
      const target = document.querySelector('.react-flow__viewport') as HTMLElement | null;
      if (!target) {
        throw new Error('react-flow viewport element not found');
      }
      const opts = {
        // PNG: white background suits design-doc embedding; SVG:
        // transparent for vector-pipeline composition.
        backgroundColor: format === 'png' ? '#ffffff' : undefined,
        width: imageWidth,
        height: imageHeight,
        style: {
          width: `${imageWidth}px`,
          height: `${imageHeight}px`,
          transform: `translate(${viewport.x}px, ${viewport.y}px) scale(${viewport.zoom})`,
        },
      };
      const dataUrl = format === 'png' ? await toPng(target, opts) : await toSvg(target, opts);
      downloadDataUrl(dataUrl, `${filename}.${format}`);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(`Export failed: ${msg}`);
    } finally {
      setBusy(null);
    }
  }

  return (
    <Panel position="top-right" className="flex flex-col items-end gap-1">
      <div className="flex gap-2">
        <button
          type="button"
          onClick={() => void exportImage('png')}
          disabled={busy !== null}
          className="rounded border border-fd-border bg-fd-card px-2 py-1 text-xs hover:bg-fd-muted disabled:opacity-50"
          aria-label="Download bridge graph as PNG"
        >
          {busy === 'png' ? 'Exporting…' : 'PNG'}
        </button>
        <button
          type="button"
          onClick={() => void exportImage('svg')}
          disabled={busy !== null}
          className="rounded border border-fd-border bg-fd-card px-2 py-1 text-xs hover:bg-fd-muted disabled:opacity-50"
          aria-label="Download bridge graph as SVG"
        >
          {busy === 'svg' ? 'Exporting…' : 'SVG'}
        </button>
      </div>
      {error ? (
        <p className="rounded bg-amber-500/20 px-2 py-1 text-xs text-amber-800 dark:text-amber-200">
          {error}
        </p>
      ) : null}
    </Panel>
  );
}
