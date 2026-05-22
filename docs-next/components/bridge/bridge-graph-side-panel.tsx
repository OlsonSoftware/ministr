// F3.6-c-ii-a — side panel rendering the export + import nodes
// of a clicked edge.
//
// Pure presentational: takes the selected edge + a node lookup map
// + a close handler. Does no fetching. F3.6-c-ii-c will add a
// source-code section that fetches via `ministr_definition` once
// F3.6-c-ii-b lands `symbol_id` on the wire shape.
//
// Layout: docks below the canvas on narrow viewports (stacked) and
// alongside on wide viewports (split). The parent owns the layout
// container; this component just renders its content.

'use client';

import { X } from 'lucide-react';

import type { LiveBridgeEdge, LiveBridgeNode } from '../landing/bridge-graph';

interface BridgeGraphSidePanelProps {
  edge: LiveBridgeEdge;
  /** Map of node id → node, so the panel can resolve `edge.from`/`edge.to`. */
  nodesById: Map<string, LiveBridgeNode>;
  onClose: () => void;
}

export function BridgeGraphSidePanel({ edge, nodesById, onClose }: BridgeGraphSidePanelProps) {
  const fromNode = nodesById.get(edge.from);
  const toNode = nodesById.get(edge.to);

  return (
    <aside className="flex flex-col gap-4 rounded border border-fd-border bg-fd-card p-4 text-sm">
      <header className="flex items-start justify-between gap-3">
        <div className="flex flex-col gap-1">
          <p className="font-mono text-xs uppercase tracking-[0.18em] text-fd-muted-foreground">
            Bridge edge
          </p>
          <h3 className="text-base font-semibold">
            <code className="font-mono">{edge.kind}</code>
          </h3>
        </div>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close side panel"
          className="rounded p-1 text-fd-muted-foreground hover:bg-fd-muted hover:text-fd-foreground"
        >
          <X className="h-4 w-4" />
        </button>
      </header>

      <EndpointBlock
        heading="Export (source)"
        node={fromNode}
        missingHint={`Node not found for edge.from = ${edge.from}`}
      />
      <EndpointBlock
        heading="Import (target)"
        node={toNode}
        missingHint={`Node not found for edge.to = ${edge.to}`}
      />

      <footer className="mt-1 text-xs text-fd-muted-foreground">
        Source viewing lands in F3.6-c-ii-c (requires F3.6-c-ii-b backend extension to expose
        symbol_id on each node).
      </footer>
    </aside>
  );
}

interface EndpointBlockProps {
  heading: string;
  node: LiveBridgeNode | undefined;
  missingHint: string;
}

function EndpointBlock({ heading, node, missingHint }: EndpointBlockProps) {
  if (!node) {
    return (
      <section>
        <h4 className="mb-1 font-mono text-xs uppercase tracking-wide text-fd-muted-foreground">
          {heading}
        </h4>
        <p className="text-xs text-amber-700 dark:text-amber-300">{missingHint}</p>
      </section>
    );
  }
  return (
    <section className="flex flex-col gap-1">
      <h4 className="font-mono text-xs uppercase tracking-wide text-fd-muted-foreground">
        {heading}
      </h4>
      <p>
        <code className="font-mono text-sm">{node.label}</code>{' '}
        <LangBadge lang={node.lang} />
      </p>
      {node.file ? (
        <p className="font-mono text-xs text-fd-muted-foreground">
          {node.file}
          {node.angle !== undefined ? null : (
            <>
              {/* The marketing sample omits line numbers; live data carries them via the BridgeNode wire shape. */}
            </>
          )}
        </p>
      ) : (
        <p className="text-xs text-fd-muted-foreground italic">no file in this node</p>
      )}
    </section>
  );
}

function LangBadge({ lang }: { lang: string }) {
  return (
    <span className="ml-1 inline-flex items-center rounded-full border border-fd-border bg-fd-muted/50 px-2 py-0.5 font-mono text-xs">
      {lang}
    </span>
  );
}
