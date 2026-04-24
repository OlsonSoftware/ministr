import { Reveal } from '@/components/landing/reveal';

/**
 * StatsStrip — horizontal trust band under the hero.
 * Two concrete, verifiable facts. Symmetric hairlines above and below.
 */
const STATS = [
  { k: '0', u: 'API calls', sub: '100% local — embeddings, index, storage' },
  { k: '60–80%', u: 'compression', sub: 'automatic at 80% budget pressure' },
] as const;

export function StatsStrip() {
  return (
    <section className="relative py-10 sm:py-12">
      <div className="ministr-spectrum-rule mx-auto max-w-6xl" />
      <div className="mx-auto grid max-w-5xl grid-cols-1 gap-8 px-6 py-10 sm:grid-cols-2 sm:gap-6">
        {STATS.map((s, i) => (
          <Reveal key={s.u} delay={i * 0.08}>
            <div className="flex flex-col gap-1">
              <span className="font-mono text-[clamp(1.6rem,2.6vw,2.25rem)] font-semibold tracking-tight text-fd-foreground tabular-nums">
                {s.k}
              </span>
              <span className="text-[11px] font-mono uppercase tracking-[0.18em] text-[var(--ministr-accent-text)]">
                {s.u}
              </span>
              <span className="ministr-body-quiet text-[12.5px]">{s.sub}</span>
            </div>
          </Reveal>
        ))}
      </div>
      <div className="ministr-spectrum-rule mx-auto max-w-6xl" />
    </section>
  );
}
