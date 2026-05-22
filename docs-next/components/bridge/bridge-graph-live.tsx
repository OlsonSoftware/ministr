// F3.6-b-ii-b — live-fetch wrapper for the bridge-graph visualizer.
//
// Reads `?api=&id=&token=` from the URL and fetches the F3.6-a
// endpoint `{api}/api/v1/corpora/{id}/bridge/graph` on mount. Falls
// back to the caller-supplied `defaultData` (typically the F2.5
// sample) on:
// - missing `api` or `id` params (default flow)
// - fetch failure (CORS, 4xx, 5xx, network)
// - malformed JSON shape
//
// `token` is optional; when present it's sent as
// `Authorization: Bearer <token>` for hitting authenticated
// cloud endpoints. Self-hosted daemons running without OAuth ignore
// it.
//
// Cross-origin requests require the target daemon to have
// `MINISTR_CORS_ALLOWED_ORIGINS` set (F3.6-b-ii-a). If unset on the
// target, the fetch fails preflight and the wrapper falls back to
// the sample data + surfaces an inline error banner.

'use client';

import { useEffect, useState } from 'react';
import { useSearchParams } from 'next/navigation';

import type { LiveBridgeEdge, LiveBridgeNode } from '../landing/bridge-graph';
import { BridgeGraphInteractive } from './bridge-graph-interactive';

interface BridgeGraphLiveProps {
  defaultData: { nodes: ReadonlyArray<LiveBridgeNode>; edges: ReadonlyArray<LiveBridgeEdge> };
}

interface LiveStatus {
  state: 'idle' | 'loading' | 'success' | 'error';
  url?: string;
  message?: string;
}

/** Wire shape returned by the F3.6-a endpoint. Extra fields (`file`,
 *  `line`, `confidence`) are tolerated by the structural typing of
 *  `LiveBridgeNode`/`LiveBridgeEdge`. */
interface LiveApiResponse {
  nodes: Array<{ id: string; label: string; lang: string; file?: string; line?: number }>;
  edges: Array<{ from: string; to: string; kind: string; confidence?: number }>;
}

function isLiveApiResponse(value: unknown): value is LiveApiResponse {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  return Array.isArray(v.nodes) && Array.isArray(v.edges);
}

export function BridgeGraphLive({ defaultData }: BridgeGraphLiveProps) {
  const params = useSearchParams();
  const api = params?.get('api') ?? null;
  const id = params?.get('id') ?? null;
  const token = params?.get('token') ?? null;

  const [data, setData] = useState<{
    nodes: ReadonlyArray<LiveBridgeNode>;
    edges: ReadonlyArray<LiveBridgeEdge>;
  }>(defaultData);
  const [status, setStatus] = useState<LiveStatus>({ state: 'idle' });

  useEffect(() => {
    if (!api || !id) {
      setStatus({ state: 'idle' });
      setData(defaultData);
      return;
    }
    const url = `${api.replace(/\/$/, '')}/api/v1/corpora/${encodeURIComponent(id)}/bridge/graph`;
    setStatus({ state: 'loading', url });

    const controller = new AbortController();
    const headers: Record<string, string> = { Accept: 'application/json' };
    if (token) headers.Authorization = `Bearer ${token}`;

    fetch(url, { headers, signal: controller.signal })
      .then(async (resp) => {
        if (!resp.ok) {
          throw new Error(`HTTP ${resp.status}`);
        }
        const json = (await resp.json()) as unknown;
        if (!isLiveApiResponse(json)) {
          throw new Error('malformed response — missing nodes/edges');
        }
        setData({ nodes: json.nodes, edges: json.edges });
        setStatus({ state: 'success', url });
      })
      .catch((err) => {
        if (err && (err as { name?: string }).name === 'AbortError') return;
        const message = err instanceof Error ? err.message : String(err);
        // Keep showing the default data when the fetch fails — the
        // sample is still useful as a "what would this look like" cue
        // while the operator debugs CORS / token / corpus-id.
        setData(defaultData);
        setStatus({ state: 'error', url, message });
      });

    return () => {
      controller.abort();
    };
  }, [api, id, token, defaultData]);

  return (
    <>
      <StatusBanner status={status} />
      <BridgeGraphInteractive data={data} />
    </>
  );
}

function StatusBanner({ status }: { status: LiveStatus }) {
  if (status.state === 'idle') return null;
  if (status.state === 'loading') {
    return (
      <p className="mb-3 rounded border border-fd-border bg-fd-muted/40 px-3 py-2 text-sm text-fd-muted-foreground">
        Fetching bridge graph from <code className="font-mono text-xs">{status.url}</code>…
      </p>
    );
  }
  if (status.state === 'success') {
    return (
      <p className="mb-3 rounded border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-700 dark:text-emerald-300">
        Live data from <code className="font-mono text-xs">{status.url}</code>
      </p>
    );
  }
  return (
    <p className="mb-3 rounded border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm text-amber-800 dark:text-amber-200">
      Couldn&apos;t reach <code className="font-mono text-xs">{status.url}</code>: {status.message}.
      Showing sample data — verify <code className="font-mono text-xs">MINISTR_CORS_ALLOWED_ORIGINS</code>{' '}
      on the target daemon and that the corpus id is valid.
    </p>
  );
}
