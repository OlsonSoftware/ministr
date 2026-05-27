// F3.6-b-i + b-ii-b — interactive bridge-graph demo page.
//
// Public-facing route at `/demo/bridge`. Renders sample data by
// default; switches to live data from a daemon when the URL carries
// `?api=&id=` (and optional `?token=` for cloud-auth endpoints).
//
// F3.6-b-iii will add the auth-gated `/orgs/{slug}/corpora/{id}/bridge`
// page once web gains authenticated routes.
//
// Why this is a separate route from `/pricing`:
//
// `/pricing` ships the static SVG `BridgeGraphHero` with zero client
// JS — part of the marketing Lighthouse ≥ 95 contract. This page
// loads `@xyflow/react` which is a DOM-only client bundle (~50KB
// gzipped). Keeping it on a separate route preserves the marketing
// path's bundle budget.

import Link from 'next/link';
import { Suspense } from 'react';

import { BridgeGraphInteractive } from '@/components/bridge/bridge-graph-interactive';
import { BridgeGraphLive } from '@/components/bridge/bridge-graph-live';
import { BRIDGE_GRAPH_SAMPLE } from '@/components/landing/bridge-graph';

export const metadata = {
  title: 'Bridge graph demo',
  description:
    'Cross-language bridge graph visualization. Zoom, pan, and hover edges to see Tauri / PyO3 / NAPI links across a polyglot codebase.',
};

export default function DemoBridgePage() {
  return (
    <main className="mx-auto flex max-w-6xl flex-col gap-8 p-8">
      <header className="flex flex-col gap-3">
        <p className="font-mono text-xs uppercase tracking-[0.18em] text-fd-muted-foreground">
          Demo · Team-tier feature preview
        </p>
        <h1 className="text-3xl font-semibold sm:text-4xl">Interactive bridge graph</h1>
        <p className="max-w-3xl text-fd-muted-foreground">
          The Team-tier web visualizer for cross-language bridge links — the same edges that the
          MIT-core <code className="font-mono">ministr_bridge</code> tool returns, rendered as an
          interactive graph. The sample below mirrors a Tauri-heavy ministr-app pattern; pass
          <code className="ml-1 font-mono">?api=&amp;id=</code> to render a live corpus.
        </p>
      </header>

      <section className="rounded-lg border border-fd-border bg-fd-card p-4">
        <Suspense
          fallback={
            <BridgeGraphInteractive
              data={BRIDGE_GRAPH_SAMPLE}
              caption={<span>Loading…</span>}
            />
          }
        >
          <BridgeGraphLive defaultData={BRIDGE_GRAPH_SAMPLE} />
        </Suspense>
      </section>

      <section className="flex flex-col gap-3 text-sm text-fd-muted-foreground">
        <h2 className="text-base font-semibold text-fd-foreground">What you&apos;re looking at</h2>
        <ul className="list-disc space-y-1 pl-5">
          <li>
            <strong className="text-fd-foreground">Nodes</strong> are individual symbols — one per
            unique <code className="font-mono">(file, symbol, line)</code> triple — colored by
            source language.
          </li>
          <li>
            <strong className="text-fd-foreground">Edges</strong> are bridge links the cross-
            language detectors produced. Edge colour encodes the bridge mechanism (Tauri command,
            PyO3 PyFunction, NAPI export, …).
          </li>
          <li>
            <strong className="text-fd-foreground">Controls</strong>: scroll or pinch to zoom; drag
            empty canvas to pan; the controls panel at the bottom-left has fit-view + zoom buttons.
          </li>
        </ul>
      </section>

      <section className="flex flex-col gap-3 rounded-lg border border-fd-border p-4 text-sm">
        <h2 className="text-base font-semibold text-fd-foreground">Wire to a live corpus</h2>
        <p className="text-fd-muted-foreground">
          Append <code className="font-mono">?api=&lt;daemon-base&gt;&amp;id=&lt;corpus-id&gt;</code>
          {' '}to the URL. The daemon must opt into CORS for this origin via{' '}
          <code className="font-mono">MINISTR_CORS_ALLOWED_ORIGINS</code>. For
          authenticated cloud endpoints, add{' '}
          <code className="font-mono">&amp;token=&lt;bearer&gt;</code>.
        </p>
        <ul className="list-disc space-y-1 pl-5 text-fd-muted-foreground">
          <li>
            <strong className="text-fd-foreground">Local daemon</strong>: start{' '}
            <code className="font-mono">ministr serve --transport http --port 3001</code> with
            <code className="ml-1 font-mono">MINISTR_CORS_ALLOWED_ORIGINS=https://ministr.ai</code>,
            then visit{' '}
            <code className="font-mono">/demo/bridge?api=http://localhost:3001&amp;id=&lt;your-corpus&gt;</code>.
          </li>
          <li>
            <strong className="text-fd-foreground">Cloud endpoint</strong>:{' '}
            <code className="font-mono">?api=https://mcp.ministr.ai&amp;id=…&amp;token=…</code>.
          </li>
        </ul>
        <p className="text-fd-muted-foreground">
          For the read-only static-export variant used on the marketing pages, see{' '}
          <Link href="/pricing" className="underline">
            /pricing
          </Link>
          .
        </p>
      </section>
    </main>
  );
}
