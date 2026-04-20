import Link from 'next/link';
import { ArrowRight, Box } from 'lucide-react';
import { SessionTrace } from '@/components/landing/session-trace';

export function Hero() {
  return (
    <section className="relative mx-auto w-full max-w-5xl px-4 sm:px-6 pt-16 pb-20 sm:pt-24 text-center">
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 -z-10 [background:radial-gradient(ellipse_70%_55%_at_50%_35%,color-mix(in_srgb,var(--color-iris-400)_18%,transparent)_0%,transparent_70%)]"
      />

      <span className="inline-flex items-center gap-1.5 rounded-full border border-fd-border bg-fd-card px-3 py-1 text-xs font-mono text-fd-muted-foreground">
        <Box className="size-3.5" aria-hidden />
        MCP server · runs locally
      </span>

      <h1 className="mt-6 text-5xl sm:text-6xl md:text-7xl font-bold tracking-tight iris-gradient-text">
        iris
      </h1>

      <p className="mx-auto mt-6 max-w-2xl text-balance text-base sm:text-lg text-fd-muted-foreground">
        Serve context to your LLM agent like an L1 cache — with session tracking,
        predictive prefetch, and budget awareness.
      </p>

      <div className="mt-8 inline-flex items-center gap-2 rounded-lg border border-fd-border bg-fd-card px-5 py-3 font-mono text-sm shadow-sm">
        <span className="text-[var(--color-iris-500)] font-semibold select-none">$</span>
        <span>claude mcp add iris -- iris</span>
      </div>

      <div className="mt-8 flex flex-wrap justify-center gap-3">
        <Link
          href="/docs/getting-started"
          className="inline-flex items-center gap-1.5 rounded-lg bg-[var(--color-iris-600)] px-5 py-2.5 text-sm font-medium text-white transition hover:bg-[var(--color-iris-700)] hover:-translate-y-px"
        >
          Get started <ArrowRight className="size-4" aria-hidden />
        </Link>
        <Link
          href="https://github.com/AlrikOlson/iris-rs"
          className="inline-flex items-center gap-1.5 rounded-lg border border-fd-border bg-fd-card px-5 py-2.5 text-sm font-medium transition hover:bg-fd-accent hover:text-fd-accent-foreground"
        >
          GitHub
        </Link>
      </div>

      <SessionTrace />

      <div className="mx-auto mt-12 grid max-w-3xl grid-cols-2 gap-6 sm:grid-cols-4">
        <StatItem value="Local" label="No API keys" />
        <StatItem value="Session-aware" label="Remembers what it sent" />
        <StatItem value="Predictive" label="Warms what's next" />
        <StatItem value="12" label="Languages indexed" />
      </div>
    </section>
  );
}

function StatItem({ value, label }: { value: string; label: string }) {
  return (
    <div className="flex flex-col items-center gap-1">
      <div className="text-xl sm:text-2xl font-semibold tracking-tight">{value}</div>
      <div className="text-xs text-fd-muted-foreground">{label}</div>
    </div>
  );
}
