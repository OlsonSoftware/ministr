'use client';

import Link from 'next/link';
import { ArrowRight, Box } from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { HeroPlayer } from '@/components/landing/hero-player';
import { GlassCard } from '@/components/landing/glass-card';
import { CopyButton } from '@/components/landing/copy-button';
import { INSTALL_COMMANDS } from '@/lib/install';
import { EASE_OUT } from '@/lib/motion';

// Hero shows the macOS one-liner (also valid on Linux). Full install
// matrix lives on /install — keep this hero terse.
const HERO_INSTALL = INSTALL_COMMANDS.find((c) => c.id === 'macos')!.command;

/**
 * Hero — headline + live asciinema recording of ministr + Claude Code.
 * The player wears a plain panel (traffic-light dots, ministr-tinted
 * surface) — asciinema's own chrome is suppressed via `app/global.css`.
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
              ministr<span className="text-[var(--color-ministr-500)]">.</span>
            </motion.h1>

            <motion.p
              {...stagger(2)}
              className="mt-6 max-w-[48ch] text-[clamp(1.125rem,1.6vw,1.375rem)] leading-snug font-medium text-fd-foreground/95"
            >
              Real codebase understanding for AI coding agents.
            </motion.p>

            <motion.p
              {...stagger(3)}
              className="ministr-body mt-4 max-w-[52ch] text-[14.5px] leading-relaxed"
            >
              Claude Code, Cursor, and Copilot explore code with grep and
              read &mdash; text matching that misses meaning and dumps whole
              files. ministr replaces that with a local code intelligence
              layer: AST-level semantic search, symbol navigation, real
              reference graphs, and cross-language bridge detection across
              ~29 languages. As a bonus, it remembers what the agent has
              seen and ships only what changed next turn.
            </motion.p>

            <motion.div {...stagger(4)} className="mt-8 flex flex-wrap items-center gap-3">
              <Link
                href="/install"
                className="ministr-cta-primary group inline-flex items-center gap-1.5 rounded-lg px-5 py-3 text-[15px] font-semibold"
              >
                Download the installer
                <ArrowRight className="size-4 transition-transform group-hover:translate-x-0.5" aria-hidden />
              </Link>
              <Link
                href="/docs/architecture"
                className="inline-flex items-center gap-1.5 rounded-lg border border-fd-border/70 bg-[color-mix(in_oklch,var(--ministr-surface)_55%,transparent)] px-4 py-2.5 text-sm font-medium text-fd-foreground backdrop-blur transition hover:bg-[color-mix(in_oklch,var(--ministr-surface)_75%,transparent)]"
              >
                Read the architecture
              </Link>
            </motion.div>

            <motion.div {...stagger(5)} className="mt-5">
              <p className="ministr-body-quiet text-[12px]">
                One installer for macOS, Windows &amp; Linux — adds the{' '}
                <span className="font-mono text-fd-foreground">ministr</span>{' '}
                CLI to your PATH automatically. Just need the CLI?
              </p>
              <GlassCard padded={false} className="mt-2 inline-flex items-center gap-3 pl-4 pr-2 py-1.5 font-mono text-xs">
                <span className="text-[var(--color-ministr-400)] select-none">$</span>
                <span>{HERO_INSTALL}</span>
                <CopyButton
                  value={HERO_INSTALL}
                  label="Copy CLI install command"
                  size="sm"
                  className="ml-1"
                />
              </GlassCard>
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
