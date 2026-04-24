import { Reveal } from '@/components/landing/reveal';

/**
 * StatsStrip — horizontal F-pattern trust band under the hero.
 * Mono numerals, spectrum hairlines top & bottom.
 */
const STATS = [
  { k: '0', u: 'API calls', sub: '100% local' },
  { k: '60–80%', u: 'compression', sub: 'at pressure' },
  { k: '∞', u: 'MCP clients', sub: 'Claude · Cursor · Copilot' },
  { k: 'Rust', u: 'zero unsafe', sub: 'static-export docs' },
] as const;

export function StatsStrip() {
  return (
    <section className="relative py-10 sm:py-12">
      <div className="ministr-spectrum-rule mx-auto max-w-6xl" />
      <div className="mx-auto grid max-w-6xl grid-cols-2 gap-6 px-6 py-8 sm:grid-cols-4 sm:gap-4">
        {STATS.map((s, i) => (
          <Reveal key={s.u} delay={i * 0.08}>
            <div className="flex flex-col gap-1">
              <span className="font-mono text-[clamp(1.5rem,2.4vw,2rem)] font-semibold tracking-tight text-fd-foreground tabular-nums">
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
