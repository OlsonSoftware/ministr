import { Reveal } from '@/components/landing/reveal';

const CLAIMS = [
  { struck: 'a vector database', answer: 'ministr answers what this agent needs right now — not what vectors look similar.' },
  { struck: 'an agent runtime',  answer: 'ministr is a sidecar. You keep your agent. You keep your stack.' },
  { struck: 'a token proxy',     answer: 'ministr operates at the knowledge layer. It decides what becomes tokens, not which ones to evict.' },
  { struck: 'classical RAG',     answer: 'ministr is stateful across turns. It tracks what was delivered and serves only the delta.' },
] as const;

/**
 * WhatMinistrIsnt — center-aligned strikethroughs. Differentiation via
 * declarative negation. Ben-David 2026 notes this pattern creates an
 * attention hotspot for landing pages with simpler visual balance.
 */
export function WhatMinistrIsnt() {
  return (
    <section className="relative py-24 sm:py-32">
      <div className="mx-auto w-full max-w-5xl px-4 text-center sm:px-6">
        <Reveal>
          <p className="ministr-eyebrow justify-center">What ministr is not</p>
        </Reveal>
        <Reveal delay={0.08}>
          <h2 className="mt-4 text-[clamp(2rem,4vw,3rem)] font-semibold leading-[1.1] tracking-tight text-fd-foreground">
            Not another box on the diagram.
          </h2>
        </Reveal>

        <ul className="mt-14 flex flex-col items-center gap-10">
          {CLAIMS.map((c, i) => (
            <Reveal as="li" key={c.struck} delay={0.12 + i * 0.08} className="max-w-[52ch]">
              <p className="text-[clamp(1.5rem,2.6vw,2rem)] font-semibold leading-tight text-fd-foreground">
                ministr is not{' '}
                <span className="strike-claim">{c.struck}</span>.
              </p>
              <p className="ministr-body mt-3 text-[15.5px] leading-relaxed">
                {c.answer}
              </p>
            </Reveal>
          ))}
        </ul>
      </div>
    </section>
  );
}
