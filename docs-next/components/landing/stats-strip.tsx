/**
 * StatsStrip — horizontal band under the hero. Two concrete claims
 * about what ministr does, not estimated percentages.
 *
 * The earlier "60–80% fewer tokens" headline ran straight into the
 * Thesis WasteDiagram below, which computes a ~96% / 22× reduction
 * from its worked example; the reader would see two different
 * numbers for the same claim. Dropping the percentage avoids the
 * conflict and lets the diagram do the quantitative work.
 */
const STATS = [
  { k: '0', u: 'API calls', sub: 'embeddings, index, and storage stay on your machine' },
  { k: 'once', u: 'per section', sub: 'ministr ships each chunk once; your agent gets a pointer next turn' },
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
