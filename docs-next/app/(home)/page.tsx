import Link from 'next/link';
import { ArrowRight } from 'lucide-react';
import { Hero } from '@/components/landing/hero';
import { ChromaticFlowClient } from '@/components/landing/chromatic-flow-client';
import { StatsStrip } from '@/components/landing/stats-strip';
import { Thesis } from '@/components/landing/thesis';
import { Mechanisms } from '@/components/landing/mechanisms';
import { ArchitectureFlow } from '@/components/landing/architecture-flow';
import { WhatMinistrIsnt } from '@/components/landing/what-ministr-isnt';
import { WorkflowComparison } from '@/components/landing/workflow-comparison';
import { InstallTabs } from '@/components/landing/install-tabs';
import { ToolList } from '@/components/landing/tool-list';
import { CtaCoda } from '@/components/landing/cta-coda';
import { NoiseOverlay } from '@/components/landing/noise-overlay';
import { Reveal } from '@/components/landing/reveal';

/**
 * Landing — the ministr "Observatory" composition.
 *
 * Flow (Z→F scan path, research-informed):
 *   1. Hero composite (aurora + lens + wordmark + live asciinema)
 *   2. Stats strip  (trust signals, F-bar)
 *   3. Thesis       (what agents waste)
 *   4. Workflow cmp (real casts, same task, two tool loadouts)
 *   5. Mechanisms   (5 mechanisms + hybrid search, bento)
 *   6. Architecture (how it wires up)
 *   7. What ministr isn't (differentiation strikethroughs)
 *   8. Install      (30-second path)
 *   9. Tool ref     (twelve tools your agent already speaks)
 *  10. CTA coda     (stop re-reading the same files)
 *  11. Footer       (retained, tightened)
 */
export default function HomePage() {
  return (
    <main
      data-ministr-landing
      className="ministr-landing relative isolate flex flex-col items-stretch overflow-x-hidden pb-0"
    >
      {/* Page-wide chromatic flow — subtle shader ambience, scroll-driven */}
      <ChromaticFlowClient />
      <NoiseOverlay />

      <Hero />
      <StatsStrip />
      <Thesis />

      <Section belowFold>
        {/*
         * Real asciinema recordings side-by-side — same prompt, two
         * tool loadouts. `hasBaseline` flips to true once
         * assets/launch-baseline.cast is recorded (see
         * scripts/demo-record-baseline-cast.sh); until then only the
         * ministr side renders.
         */}
        <WorkflowComparison hasBaseline={false} />
      </Section>

      <Mechanisms />
      <ArchitectureFlow />
      <WhatMinistrIsnt />

      <Section belowFold>
        <div className="mx-auto w-full max-w-3xl px-4 sm:px-6">
          <Reveal>
            <p className="ministr-eyebrow">Install</p>
          </Reveal>
          <Reveal delay={0.08}>
            <h2 className="mt-4 text-[clamp(2rem,4vw,3rem)] font-semibold leading-[1.05] tracking-tight text-fd-foreground">
              Install in 30 seconds.
            </h2>
          </Reveal>
          <Reveal delay={0.16}>
            <p className="ministr-body mt-4 text-[15.5px]">
              Three commands. Any MCP client. Fully local.
            </p>
          </Reveal>
        </div>
        <div className="mt-10">
          <Reveal delay={0.24}>
            <InstallTabs />
          </Reveal>
        </div>
      </Section>

      <Section belowFold>
        <div className="mx-auto w-full max-w-3xl px-4 sm:px-6">
          <Reveal>
            <p className="ministr-eyebrow">Tools</p>
          </Reveal>
          <Reveal delay={0.08}>
            <h2 className="mt-4 text-[clamp(2rem,4vw,3rem)] font-semibold leading-[1.05] tracking-tight text-fd-foreground">
              Twelve tools your agent already speaks.
            </h2>
          </Reveal>
          <Reveal delay={0.16}>
            <p className="ministr-body mt-4 text-[15.5px]">
              ministr exposes these as MCP tools. Every one links to its doc page.
            </p>
          </Reveal>
          <div className="ministr-spectrum-rule mt-10" />
          <div className="mt-10">
            <Reveal delay={0.24}>
              <ToolList />
            </Reveal>
          </div>
        </div>
      </Section>

      <CtaCoda />

      <Section tight belowFold>
        <div className="mx-auto w-full max-w-3xl px-4 sm:px-6">
          <div className="flex flex-col items-center gap-4 pt-10 text-center">
            {/* Spectrum hairline separator — ties into the brand material set */}
            <div className="ministr-spectrum-rule-major w-full max-w-md" />
            <p className="ministr-body-quiet inline-flex items-center gap-2 text-[13px]">
              <span aria-hidden className="ministr-mark-dot" />
              Full docs cover every tool, every config key, and the architecture in depth.
            </p>
            <nav className="flex flex-wrap justify-center gap-x-6 gap-y-2 text-[14px]">
              <FooterLink href="/docs/getting-started">
                Getting started
                <ArrowRight className="ml-1 size-3.5" aria-hidden />
              </FooterLink>
              <FooterLink href="/docs/tools">Tool reference</FooterLink>
              <FooterLink href="/docs/architecture">Architecture</FooterLink>
              <FooterLink href="/docs/concepts">Concepts</FooterLink>
              <FooterLink href="https://github.com/AlrikOlson/ministr-rs">GitHub</FooterLink>
            </nav>
          </div>
        </div>
      </Section>
    </main>
  );
}

function Section({
  children,
  tight,
  belowFold,
}: {
  children: React.ReactNode;
  tight?: boolean;
  belowFold?: boolean;
}) {
  const pad = tight ? 'py-10 sm:py-12' : 'py-20 sm:py-24';
  const cv = belowFold ? ' below-fold' : '';
  return <section className={'relative ' + pad + cv}>{children}</section>;
}

function FooterLink({
  href,
  children,
}: {
  href: string;
  children: React.ReactNode;
}) {
  return (
    <Link
      href={href}
      className="group inline-flex items-center text-fd-muted-foreground transition hover:text-[var(--ministr-accent-text)]"
    >
      {children}
    </Link>
  );
}
