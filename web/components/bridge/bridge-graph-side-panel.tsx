// F3.6-c-ii-a + F3.6-c-ii-c — side panel for a clicked edge.
//
// F3.6-c-ii-a: shows metadata (label, language, file) for each
// endpoint.
// F3.6-c-ii-c: when an `apiContext` is provided (the demo page is
// running with `?api=&id=` query params), fetches
// `{api}/api/v1/corpora/{id}/definition/{symbol_id}` per endpoint
// that has a `symbol_id` and renders the source.
//
// Failure modes (any combination handled gracefully):
// - No apiContext (sample mode) → metadata only, "demo mode" hint.
// - apiContext present + endpoint has no symbol_id → metadata + "no
//   indexed symbol for this endpoint" hint.
// - Fetch fails → metadata + amber inline error.
// - Both present + fetch ok → metadata + full source pane.

'use client';

import { X } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';

import type { LiveBridgeEdge, LiveBridgeNode } from '../landing/bridge-graph';

export interface ApiContext {
  api: string;
  id: string;
  token: string | null;
}

interface BridgeGraphSidePanelProps {
  edge: LiveBridgeEdge;
  /** Map of node id → node, so the panel can resolve `edge.from`/`edge.to`. */
  nodesById: Map<string, LiveBridgeNode>;
  /** F3.6-c-ii-c — when non-null, the panel fetches source per endpoint. */
  apiContext: ApiContext | null;
  onClose: () => void;
}

type SourceState =
  | { kind: 'idle' }
  | { kind: 'loading' }
  | { kind: 'success'; text: string }
  | { kind: 'error'; message: string };

interface DefinitionResponse {
  /** Field carried by `ministr_api::query::SymbolDefinition`. */
  source_context?: string;
}

function isDefinitionResponse(v: unknown): v is DefinitionResponse {
  if (typeof v !== 'object' || v === null) return false;
  const obj = v as Record<string, unknown>;
  return obj.source_context === undefined || typeof obj.source_context === 'string';
}

export function BridgeGraphSidePanel({
  edge,
  nodesById,
  apiContext,
  onClose,
}: BridgeGraphSidePanelProps) {
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
        apiContext={apiContext}
        missingHint={`Node not found for edge.from = ${edge.from}`}
      />
      <EndpointBlock
        heading="Import (target)"
        node={toNode}
        apiContext={apiContext}
        missingHint={`Node not found for edge.to = ${edge.to}`}
      />

      {!apiContext ? (
        <footer className="mt-1 text-xs text-fd-muted-foreground">
          Sample mode — pass{' '}
          <code className="font-mono text-xs">?api=&amp;id=</code> to fetch source via{' '}
          <code className="font-mono text-xs">ministr_definition</code>.
        </footer>
      ) : null}
    </aside>
  );
}

interface EndpointBlockProps {
  heading: string;
  node: LiveBridgeNode | undefined;
  apiContext: ApiContext | null;
  missingHint: string;
}

function EndpointBlock({ heading, node, apiContext, missingHint }: EndpointBlockProps) {
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
        <p className="font-mono text-xs text-fd-muted-foreground">{node.file}</p>
      ) : (
        <p className="text-xs italic text-fd-muted-foreground">no file in this node</p>
      )}
      <SourceViewer node={node} apiContext={apiContext} />
    </section>
  );
}

function SourceViewer({
  node,
  apiContext,
}: {
  node: LiveBridgeNode;
  apiContext: ApiContext | null;
}) {
  // Only fetch when both the live context AND a resolved symbol_id
  // are present. Otherwise the metadata block stands on its own.
  const shouldFetch = useMemo(
    () => apiContext !== null && typeof node.symbol_id === 'string' && node.symbol_id.length > 0,
    [apiContext, node.symbol_id],
  );

  const [state, setState] = useState<SourceState>({ kind: 'idle' });

  useEffect(() => {
    if (!shouldFetch || !apiContext || !node.symbol_id) {
      setState({ kind: 'idle' });
      return;
    }
    const controller = new AbortController();
    const url = `${apiContext.api.replace(/\/$/, '')}/api/v1/corpora/${encodeURIComponent(
      apiContext.id,
    )}/definition/${encodeURIComponent(node.symbol_id)}`;
    const headers: Record<string, string> = { Accept: 'application/json' };
    if (apiContext.token) headers.Authorization = `Bearer ${apiContext.token}`;

    setState({ kind: 'loading' });
    fetch(url, { headers, signal: controller.signal })
      .then(async (resp) => {
        if (!resp.ok) {
          throw new Error(`HTTP ${resp.status}`);
        }
        const json = (await resp.json()) as unknown;
        if (!isDefinitionResponse(json)) {
          throw new Error('malformed response — missing source_context');
        }
        const text = json.source_context ?? '';
        setState({ kind: 'success', text });
      })
      .catch((err) => {
        if (err && (err as { name?: string }).name === 'AbortError') return;
        const message = err instanceof Error ? err.message : String(err);
        setState({ kind: 'error', message });
      });

    return () => {
      controller.abort();
    };
  }, [shouldFetch, apiContext, node.symbol_id]);

  if (!apiContext) return null;
  if (!node.symbol_id) {
    return (
      <p className="mt-1 text-xs italic text-fd-muted-foreground">
        No indexed symbol for this endpoint — the symbol indexer hadn&apos;t covered{' '}
        <code className="font-mono">{node.file ?? 'this file'}</code> when the bridge was
        extracted.
      </p>
    );
  }
  if (state.kind === 'idle' || state.kind === 'loading') {
    return (
      <p className="mt-1 text-xs text-fd-muted-foreground">Fetching source…</p>
    );
  }
  if (state.kind === 'error') {
    return (
      <p className="mt-1 rounded bg-amber-500/20 px-2 py-1 text-xs text-amber-800 dark:text-amber-200">
        Source fetch failed: {state.message}
      </p>
    );
  }
  // Success
  if (!state.text) {
    return (
      <p className="mt-1 text-xs italic text-fd-muted-foreground">
        Symbol resolved but source_context was empty.
      </p>
    );
  }
  return (
    <pre className="mt-1 max-h-96 overflow-auto rounded border border-fd-border bg-fd-muted/30 p-2 text-xs">
      <code className="font-mono">{state.text}</code>
    </pre>
  );
}

function LangBadge({ lang }: { lang: string }) {
  return (
    <span className="ml-1 inline-flex items-center rounded-full border border-fd-border bg-fd-muted/50 px-2 py-0.5 font-mono text-xs">
      {lang}
    </span>
  );
}
