import Link from 'next/link';
import { ArrowRight } from 'lucide-react';
import { Hero } from '@/components/landing/hero';
import { StatsStrip } from '@/components/landing/stats-strip';
import { Thesis } from '@/components/landing/thesis';
import { Mechanisms } from '@/components/landing/mechanisms';
import { ArchitectureFlow } from '@/components/landing/architecture-flow';
import { InstallTabs } from '@/components/landing/install-tabs';
import { ToolList } from '@/components/landing/tool-list';
import { CtaCoda } from '@/components/landing/cta-coda';

export default function HomePage() {
  return (
    <main
      data-ministr-landing
      className="ministr-landing relative isolate flex flex-col items-stretch overflow-x-hidden pb-0"
    >
      <Hero />
      <StatsStrip />
      <Thesis />
      <Mechanisms />
      <ArchitectureFlow />

      <Section belowFold>
        <div className="mx-auto w-full max-w-3xl px-4 sm:px-6">
          <p className="ministr-eyebrow">Install</p>
          <h2 className="mt-4 text-[clamp(2rem,4vw,3rem)] font-semibold leading-[1.05] tracking-tight text-fd-foreground">
            Install in 30 seconds.
          </h2>
          <p className="ministr-body mt-4 text-[15.5px]">
            Three commands. Any MCP client. Fully local.
          </p>
        </div>
        <div className="mt-10">
          <InstallTabs />
        </div>
      </Section>

      <Section belowFold>
        <div className="mx-auto w-full max-w-3xl px-4 sm:px-6">
          <p className="ministr-eyebrow">Tools</p>
          <h2 className="mt-4 text-[clamp(2rem,4vw,3rem)] font-semibold leading-[1.05] tracking-tight text-fd-foreground">
            Fifteen tools for your agent.
          </h2>
          <p className="ministr-body mt-4 text-[15.5px]">
            Exposed as MCP tools. Every one links to its doc page.
          </p>
          <div className="ministr-spectrum-rule mt-10" />
          <div className="mt-10">
            <ToolList />
          </div>
        </div>
      </Section>

      <CtaCoda />

      <Section tight belowFold>
        <div className="mx-auto w-full max-w-3xl px-4 sm:px-6">
          <div className="flex flex-col items-center gap-4 pt-10 text-center">
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
