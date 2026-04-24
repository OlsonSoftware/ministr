'use client';

import Link from 'next/link';
import { ArrowRight, Box } from 'lucide-react';
import { GithubGlyph } from '@/components/landing/github-glyph';
import { motion, useReducedMotion } from 'motion/react';
import { HeroPlayer } from '@/components/landing/hero-player';
import { GlassCard } from '@/components/landing/glass-card';
import { CopyButton } from '@/components/landing/copy-button';
import { EASE_OUT } from '@/lib/motion';

/**
 * Hero — headline + live asciinema recording of ministr + Claude Code,
 * layered over the page-wide chromatic-flow shader. The player wears
 * the same Shell as the old SessionTrace simulation (traffic-light
 * dots, ministr-tinted surface, glow shadow) — asciinema's own chrome
 * is suppressed via `app/global.css` overrides.
 */
export function Hero() {
  const reduced = useReducedMotion();
  const stagger = (i: number) => ({
    initial: reduced ? false : { opacity: 0, y: 14 },
    animate: reduced ? undefined : { opacity: 1, y: 0 },
    transition: { duration: 0.7, ease: EASE_OUT, delay: 0.08 * i },
  });

  return (
    <section className="relative w-full pt-24 pb-24 sm:pt-28 sm:pb-28">
      <div className="relative mx-auto w-full max-w-6xl px-4 sm:px-6">
        <div className="grid items-center gap-12 lg:grid-cols-[minmax(0,_1fr)_minmax(0,_1.1fr)] lg:gap-14">
          <div className="relative">
            <motion.span
              {...stagger(0)}
              className="inline-flex items-center gap-2 rounded-full border border-[color-mix(in_oklch,var(--color-ministr-400)_28%,transparent)] bg-[color-mix(in_oklch,var(--ministr-surface)_60%,transparent)] px-3 py-1 text-[11px] font-mono text-fd-muted-foreground backdrop-blur"
            >
              <Box className="size-3.5 text-[var(--color-ministr-400)]" aria-hidden />
              MCP server · local · Rust
            </motion.span>

            <motion.h1
              {...stagger(1)}
              className="ministr-hero-mark mt-6 text-[clamp(3.25rem,9vw,6.75rem)] font-semibold leading-[0.9] text-fd-foreground"
            >
              ministr<span className="ministr-gradient-text">.</span>
            </motion.h1>

            <motion.p
              {...stagger(2)}
              className="mt-6 max-w-[48ch] text-[clamp(1.125rem,1.6vw,1.375rem)] leading-snug font-medium text-fd-foreground/95"
            >
              An L1 cache for your agent&rsquo;s context.
            </motion.p>

            <motion.p
              {...stagger(3)}
              className="ministr-body mt-4 max-w-[52ch] text-[14.5px] leading-relaxed"
            >
              Serve what&rsquo;s needed. Remember what&rsquo;s been sent.
              Predict what comes next. Semantic search, symbol navigation,
              and cross-language bridges over your files — running as a
              single local process, state in a SQLite file.
            </motion.p>

            <motion.div {...stagger(4)} className="mt-8">
              <GlassCard padded={false} className="inline-flex items-center gap-3 pl-4 pr-2 py-2 font-mono text-sm">
                <span className="text-[var(--color-ministr-400)] select-none">$</span>
                <span>claude mcp add ministr -- ministr</span>
                <CopyButton
                  value="claude mcp add ministr -- ministr"
                  label="Copy install command"
                  size="sm"
                  className="ml-1"
                />
              </GlassCard>
            </motion.div>

            <motion.div {...stagger(5)} className="mt-6 flex flex-wrap items-center gap-3">
              <Link
                href="/docs/getting-started"
                className="ministr-cta-primary group inline-flex items-center gap-1.5 rounded-lg px-4 py-2.5 text-sm font-medium"
              >
                Install ministr
                <ArrowRight className="size-4 transition-transform group-hover:translate-x-0.5" aria-hidden />
              </Link>
              <Link
                href="/docs/architecture"
                className="inline-flex items-center gap-1.5 rounded-lg border border-fd-border/70 bg-[color-mix(in_oklch,var(--ministr-surface)_55%,transparent)] px-4 py-2.5 text-sm font-medium text-fd-foreground backdrop-blur transition hover:bg-[color-mix(in_oklch,var(--ministr-surface)_75%,transparent)]"
              >
                Read the architecture
              </Link>
              <Link
                href="https://github.com/OlsonSoftware/ministr"
                className="inline-flex items-center gap-1.5 text-sm font-medium text-fd-muted-foreground transition hover:text-[var(--ministr-accent-text)]"
              >
                <GithubGlyph className="size-4" />
                GitHub
              </Link>
            </motion.div>
          </div>

          <motion.div
            initial={reduced ? false : { opacity: 0, scale: 0.97 }}
            animate={reduced ? undefined : { opacity: 1, scale: 1 }}
            transition={{ duration: 0.9, ease: EASE_OUT, delay: 0.4 }}
            className="relative"
          >
            <GlassCard padded={false} className="p-2 sm:p-3">
              <HeroPlayer />
            </GlassCard>
          </motion.div>
        </div>
      </div>
    </section>
  );
}
