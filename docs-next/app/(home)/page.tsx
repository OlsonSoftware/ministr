import Link from 'next/link';
import {
  ArrowRight,
  Braces,
  Cpu,
  Eye,
  EyeOff,
  Gauge,
  GitBranch,
  History,
  Layers,
  Package,
  Puzzle,
  Repeat,
  SearchCode,
  Sparkles,
  Squircle,
  TerminalSquare,
  TreePine,
  Wand2,
  Waypoints,
} from 'lucide-react';
import { Hero } from '@/components/landing/hero';
import { SectionHeader } from '@/components/landing/section-header';
import { FeatureCard } from '@/components/landing/feature-card';
import { LanguageChips } from '@/components/landing/language-chips';
import { ArchitectureDiagram } from '@/components/landing/architecture-diagram';
import { BridgesDiagram } from '@/components/landing/bridges-diagram';
import { CompareTable } from '@/components/landing/compare-table';
import { InstallTabs } from '@/components/landing/install-tabs';
import { ObservatoryPreview } from '@/components/landing/observatory-preview';

export default function HomePage() {
  return (
    <main className="flex flex-col items-stretch pb-24">
      <Hero />

      <section className="mx-auto w-full max-w-5xl px-4 sm:px-6 mt-8">
        <SectionHeader
          eyebrow="Problem"
          eyebrowIcon={<Sparkles className="size-3.5" aria-hidden />}
          title="Why iris"
          subtitle="LLM agents waste most of their context window. iris fixes the three root causes."
        />
        <div className="grid gap-4 sm:grid-cols-3">
          <FeatureCard
            icon={<Repeat className="size-5" aria-hidden />}
            title="Re-reading"
            body="Agents fetch the same file over and over. iris remembers what it sent this session and deduplicates. When a section changes, it delivers only the delta."
          />
          <FeatureCard
            icon={<EyeOff className="size-5" aria-hidden />}
            title="Blind retrieval"
            body={<>
              <code className="font-mono text-[0.9em]">grep</code> +{' '}
              <code className="font-mono text-[0.9em]">cat</code>{' '}burn tokens on code that isn&apos;t relevant. iris indexes
              your corpus semantically and returns just the piece that answers the question.
            </>}
          />
          <FeatureCard
            icon={<Wand2 className="size-5" aria-hidden />}
            title="Cold reads"
            body="Every new fetch is a round trip your agent waits on. iris predicts what it's going to ask for next and warms it in the background."
          />
        </div>
      </section>

      <section className="mt-24">
        <SectionHeader
          eyebrow="Architecture"
          eyebrowIcon={<Squircle className="size-3.5" aria-hidden />}
          title="How it fits together"
          subtitle="One local binary sits between your MCP client and your files."
        />
        <ArchitectureDiagram />
      </section>

      <section className="mx-auto w-full max-w-5xl px-4 sm:px-6 mt-24">
        <SectionHeader
          eyebrow="Capabilities"
          eyebrowIcon={<Gauge className="size-3.5" aria-hidden />}
          title="What iris does"
          subtitle="One local binary, no API keys."
        />
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          <FeatureCard
            icon={<SearchCode className="size-5" aria-hidden />}
            title="Semantic search"
            body="Search across your codebase and docs by meaning, not just text. Returns the specific section that answers the question."
          />
          <FeatureCard
            icon={<Braces className="size-5" aria-hidden />}
            title="Code symbol navigation"
            body="Find and trace functions, types, and callers across your project — not just file-level matches."
          />
          <FeatureCard
            icon={<Waypoints className="size-5" aria-hidden />}
            title="Cross-language bridges"
            body="Follows function calls where one language hands off to another — Rust to JavaScript, Python to Rust, front-end to back-end."
          />
          <FeatureCard
            icon={<History className="size-5" aria-hidden />}
            title="Session tracking"
            body="Remembers what it sent your agent this session. Skips repeats; ships only the changed part when a section is edited."
          />
          <FeatureCard
            icon={<Gauge className="size-5" aria-hidden />}
            title="Budget awareness"
            body="Tracks context-window usage. When it fills up, older material gets compressed instead of silently dropping."
          />
          <FeatureCard
            icon={<Cpu className="size-5" aria-hidden />}
            title="Local embeddings"
            body="Embeddings run on your machine, not a third-party API. No network calls, no API keys, no tokens leaving the box."
          />
        </div>
      </section>

      <section className="mt-24">
        <SectionHeader
          eyebrow="Desktop app"
          eyebrowIcon={<Eye className="size-3.5" aria-hidden />}
          title="The observatory, for when you want to watch"
          subtitle="A desktop companion that attaches to the same local daemon your agents use — inspect corpora, replay sessions, and tune configuration without leaving the GUI."
        />
        <ObservatoryPreview />
        <div className="mx-auto mt-8 grid w-full max-w-5xl gap-4 px-4 sm:grid-cols-2 sm:px-6 lg:grid-cols-3">
          <FeatureCard
            icon={<Layers className="size-5" aria-hidden />}
            title="Overview"
            body="Live counts of files, code symbols, and active sessions across every registered corpus — plus a live feed of what's being indexed."
          />
          <FeatureCard
            icon={<SearchCode className="size-5" aria-hidden />}
            title="Query playground"
            body="Run iris_survey, iris_symbols, iris_definition, and iris_references against any registered corpus. See the same ranked results your agent sees."
          />
          <FeatureCard
            icon={<Gauge className="size-5" aria-hidden />}
            title="Session dashboard"
            body="Replay a session turn by turn: which sections were delivered, which got evicted, and how the budget tracked across the conversation."
          />
          <FeatureCard
            icon={<GitBranch className="size-5" aria-hidden />}
            title="Symbol graph"
            body="An interactive map of your codebase as a collapsible graph. Navigate callers, implementors, and cross-language bridges visually."
          />
          <FeatureCard
            icon={<TreePine className="size-5" aria-hidden />}
            title="Corpus treemap"
            body="Treemap of disk and token footprint per path. Spot a runaway directory before it bloats your index."
          />
          <FeatureCard
            icon={<TerminalSquare className="size-5" aria-hidden />}
            title="Log viewer + settings"
            body="Tail daemon logs with filtering, and tune budget, prefetch, and embedding settings from the UI — changes apply without a restart."
          />
        </div>
      </section>

      <section className="mt-24">
        <SectionHeader
          eyebrow="Bridges"
          eyebrowIcon={<Puzzle className="size-3.5" aria-hidden />}
          title="Cross-language bridges"
          subtitle="Trace function calls across language boundaries automatically."
        />
        <BridgesDiagram />
      </section>

      <section className="mx-auto w-full max-w-4xl px-4 sm:px-6 mt-24">
        <SectionHeader
          eyebrow="Languages"
          eyebrowIcon={<Layers className="size-3.5" aria-hidden />}
          title="Twelve languages, one symbol index"
          subtitle="Real parsers, not regex — symbol extraction, reference tracing, and bridge detection across the stack."
        />
        <LanguageChips />
      </section>

      <section className="mt-24">
        <SectionHeader
          eyebrow="Comparison"
          eyebrowIcon={<Sparkles className="size-3.5" aria-hidden />}
          title="How it compares"
          subtitle="iris isn't a vector DB, a RAG framework, or a search tool. It's a stateful, cache-aware context source exposed as MCP tools."
        />
        <CompareTable />
      </section>

      <section className="mt-24">
        <SectionHeader
          eyebrow="Install"
          eyebrowIcon={<Package className="size-3.5" aria-hidden />}
          title="Get started in 30 seconds"
          subtitle="Install the CLI, wire it into your agent. No API keys, no service."
        />
        <InstallTabs />
      </section>

      <section className="mx-auto w-full max-w-3xl px-4 sm:px-6 mt-24 text-center">
        <h2 className="text-2xl sm:text-3xl font-semibold tracking-tight">Dig deeper</h2>
        <div className="mt-6 flex flex-wrap justify-center gap-3">
          <Link
            href="/docs/getting-started"
            className="inline-flex items-center gap-1.5 rounded-lg bg-[var(--color-iris-600)] px-4 py-2 text-sm font-medium text-white transition hover:bg-[var(--color-iris-700)]"
          >
            Get started <ArrowRight className="size-4" aria-hidden />
          </Link>
          <Link
            href="/docs/tools"
            className="inline-flex items-center gap-1.5 rounded-lg border border-fd-border bg-fd-card px-4 py-2 text-sm font-medium transition hover:bg-fd-accent hover:text-fd-accent-foreground"
          >
            Tool reference
          </Link>
          <Link
            href="/docs/architecture"
            className="inline-flex items-center gap-1.5 rounded-lg border border-fd-border bg-fd-card px-4 py-2 text-sm font-medium transition hover:bg-fd-accent hover:text-fd-accent-foreground"
          >
            Architecture
          </Link>
        </div>
      </section>
    </main>
  );
}
