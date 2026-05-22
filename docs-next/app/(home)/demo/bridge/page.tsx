// F3.6-b-i — interactive bridge-graph demo page.
//
// Public-facing route at `/demo/bridge` that renders the
// [`BridgeGraphInteractive`] component against the F2.5
// `BRIDGE_GRAPH_SAMPLE` so the Team-tier visualizer is observable
// without auth scaffolding. F3.6-b-ii will switch the data source to
// live-fetch from `/api/v1/corpora/{id}/bridge/graph`; F3.6-b-iii
// adds the auth-gated `/orgs/{slug}/corpora/{id}/bridge` page.
//
// Why this is a separate route from `/pricing`:
//
// `/pricing` ships the static SVG `BridgeGraphHero` with zero client
// JS — part of the marketing Lighthouse ≥ 95 contract. This page
// loads `@xyflow/react` which is a DOM-only client bundle (~50KB
// gzipped). Keeping it on a separate route preserves the marketing
// path's bundle budget.

import Link from 'next/link';

import { BridgeGraphInteractive } from '@/components/bridge/bridge-graph-interactive';
import { BRIDGE_GRAPH_SAMPLE } from '@/components/landing/bridge-graph';

export const metadata = {
  title: 'Bridge graph — interactive demo · ministr',
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
          interactive graph. The sample below mirrors a Tauri-heavy ministr-app pattern.
        </p>
      </header>

      <section className="rounded-lg border border-fd-border bg-fd-card p-4">
        <BridgeGraphInteractive
          data={BRIDGE_GRAPH_SAMPLE}
          caption={
            <span>
              Sample data. Live corpus rendering lands on{' '}
              <code className="font-mono">/orgs/&lt;slug&gt;/corpora/&lt;id&gt;/bridge</code> once
              docs-next gains authenticated routes (F3.6-b-iii).
            </span>
          }
        />
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
        <p>
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
