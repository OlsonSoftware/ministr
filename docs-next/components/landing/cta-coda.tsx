import Link from 'next/link';
import { ArrowRight } from 'lucide-react';
import { GithubGlyph } from '@/components/landing/github-glyph';
import { Reveal } from '@/components/landing/reveal';

/**
 * CtaCoda — big closing coda. Last thing a visitor sees before the
 * footer; the whole page funnels into it. Ambient comes from the
 * page-wide chromatic-flow shader — no local backdrop.
 */
export function CtaCoda() {
  return (
    <section className="relative py-28 sm:py-36">
      <div className="relative mx-auto w-full max-w-4xl px-4 text-center sm:px-6">
        <Reveal>
          <h2 className="text-[clamp(2.5rem,6vw,4.5rem)] font-semibold leading-[1.02] tracking-tight text-fd-foreground">
            Stop re-reading the same files.
          </h2>
        </Reveal>
        <Reveal delay={0.12}>
          <p className="iris-body mx-auto mt-6 max-w-[52ch] text-[17px]">
            Install iris in 30 seconds. It works with any MCP client, runs
            100% locally, and leaves no trace on the wire.
          </p>
        </Reveal>
        <Reveal delay={0.2}>
          <div className="mt-10 flex flex-wrap justify-center gap-3">
            <Link
              href="/docs/getting-started"
              className="iris-cta-primary group inline-flex items-center gap-1.5 rounded-lg px-5 py-3 text-[15px] font-medium"
            >
              Install iris
              <ArrowRight className="size-4 transition-transform group-hover:translate-x-0.5" aria-hidden />
            </Link>
            <Link
              href="https://github.com/AlrikOlson/iris-rs"
              className="inline-flex items-center gap-1.5 rounded-lg border border-fd-border/70 bg-[color-mix(in_oklch,var(--iris-surface)_55%,transparent)] px-5 py-3 text-[15px] font-medium text-fd-foreground backdrop-blur transition hover:bg-[color-mix(in_oklch,var(--iris-surface)_78%,transparent)]"
            >
              <GithubGlyph className="size-4" />
              Star on GitHub
            </Link>
          </div>
        </Reveal>
        <Reveal delay={0.3}>
          <p className="iris-body-quiet mt-10 inline-flex items-center gap-2 text-[12.5px]">
            <span aria-hidden className="iris-mark-dot" />
            Made by Alrik · MIT license · Rust, zero unsafe.
          </p>
        </Reveal>
      </div>
    </section>
  );
}
