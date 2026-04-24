/**
 * StatsStrip — horizontal band under the hero. Two concrete,
 * verifiable facts. Symmetric hairlines above and below.
 */
const STATS = [
  { k: '0', u: 'API calls', sub: 'embeddings, index, and storage stay on your machine' },
  { k: '60–80%', u: 'fewer tokens', sub: 'your agent stops re-reading what it already has' },
] as const;

export function StatsStrip() {
  return (
    <section className="relative py-10 sm:py-12">
      <div className="ministr-spectrum-rule mx-auto max-w-6xl" />
      <div className="mx-auto grid max-w-5xl grid-cols-1 gap-8 px-6 py-10 sm:grid-cols-2 sm:gap-6">
        {STATS.map((s) => (
          <div key={s.u} className="flex flex-col gap-1">
            <span className="font-mono text-[clamp(1.6rem,2.6vw,2.25rem)] font-semibold tracking-tight text-fd-foreground tabular-nums">
              {s.k}
            </span>
            <span className="text-[11px] font-mono uppercase tracking-[0.18em] text-[var(--ministr-accent-text)]">
              {s.u}
            </span>
            <span className="ministr-body-quiet text-[12.5px]">{s.sub}</span>
          </div>
        ))}
      </div>
      <div className="ministr-spectrum-rule mx-auto max-w-6xl" />
    </section>
  );
}
