import Link from 'next/link';
import { ArrowRight } from 'lucide-react';
import { Reveal } from '@/components/landing/reveal';

/**
 * CtaCoda — closing CTA. Single primary action, plain surface.
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
          <p className="ministr-body mx-auto mt-6 max-w-[52ch] text-[17px]">
            Install in 30 seconds. Any MCP client. 100% local.
          </p>
        </Reveal>
        <Reveal delay={0.2}>
          <div className="mt-10 flex justify-center">
            <Link
              href="/docs/getting-started"
              className="ministr-cta-primary group inline-flex items-center gap-1.5 rounded-lg px-5 py-3 text-[15px] font-medium"
            >
              Install ministr
              <ArrowRight className="size-4 transition-transform group-hover:translate-x-0.5" aria-hidden />
            </Link>
          </div>
        </Reveal>
      </div>
    </section>
  );
}
