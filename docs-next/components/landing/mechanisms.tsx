import { BentoGrid, BentoTile } from '@/components/landing/bento';
import { Reveal } from '@/components/landing/reveal';

/**
 * Mechanisms — 6-tile bento grid showcasing ministr's five mechanisms
 * plus hybrid search. Each tile has a pure-CSS micro-visual and a
 * one-sentence claim.
 */
export function Mechanisms() {
  return (
    <section className="relative py-24 sm:py-32">
      <div className="mx-auto w-full max-w-6xl px-4 sm:px-6">
        <Reveal>
          <p className="ministr-eyebrow">How it works</p>
        </Reveal>
        <Reveal delay={0.08}>
          <h2 className="mt-4 text-[clamp(2rem,4vw,3rem)] font-semibold leading-[1.05] tracking-tight text-fd-foreground">
            Five mechanisms, drawn from CPU caches.
          </h2>
        </Reveal>
        <Reveal delay={0.16}>
          <p className="ministr-body mt-4 max-w-[60ch] text-[15.5px]">
            Each one solves a class of waste the agent can&rsquo;t see. They
            run together, invisibly, every time your agent asks for context.
          </p>
        </Reveal>

        <div className="mt-12">
          <BentoGrid>
            {/* Row 1: Session Shadow (7) + Delta Delivery (5) */}
            <BentoTile span={{ base: 2, md: 6, lg: 7 }}>
              <TileHeader
                kicker="Mechanism 1"
                title="Session Shadow"
                copy="A per-session timeline of every section, claim, and symbol ministr has delivered — so it knows what your agent already has."
              />
              <div className="mt-5">
                <TimelineVisual />
              </div>
            </BentoTile>

            <BentoTile span={{ base: 2, md: 6, lg: 5 }}>
              <TileHeader
                kicker="Mechanism 2"
                title="Delta Delivery"
                copy="Changed sections ship as line-level deltas. Unchanged lines stay off the wire."
              />
              <div className="mt-5">
                <DeltaVisual />
              </div>
            </BentoTile>

            {/* Row 2: Predictive Prefetch (6) + Budget & Pressure (6) */}
            <BentoTile span={{ base: 2, md: 6, lg: 6 }}>
              <TileHeader
                kicker="Mechanism 3"
                title="Predictive Prefetch"
                copy="Sequential, structural, and topical prefetch warm the likely next read — before the agent asks."
              />
              <div className="mt-5">
                <PrefetchVisual />
              </div>
            </BentoTile>

            <BentoTile span={{ base: 2, md: 6, lg: 6 }}>
              <TileHeader
                kicker="Mechanism 4"
                title="Budget & Pressure Mode"
                copy="Live token accounting. At ~80% ministr auto-compresses responses and ranks eviction candidates."
              />
              <div className="mt-5">
                <BudgetVisual />
              </div>
            </BentoTile>

            {/* Row 3: Coherence (6) + Hybrid Search (6) */}
            <BentoTile span={{ base: 2, md: 6, lg: 6 }}>
              <TileHeader
                kicker="Mechanism 5"
                title="Coherence"
                copy="When a file changes, ministr flags the delivered content as stale. No silently rotten context."
              />
              <div className="mt-5">
                <CoherenceVisual />
              </div>
            </BentoTile>

            <BentoTile span={{ base: 2, md: 6, lg: 6 }}>
              <TileHeader
                kicker="Bonus"
                title="Hybrid Search"
                copy="Dense embeddings + SPLADE sparse retrieval. Keyword + meaning, fused at rank-time."
              />
              <div className="mt-5">
                <HybridVisual />
              </div>
            </BentoTile>
          </BentoGrid>
        </div>
      </div>
    </section>
  );
}

function TileHeader({
  kicker,
  title,
  copy,
}: {
  kicker: string;
  title: string;
  copy: string;
}) {
  return (
    <>
      <p className="ministr-eyebrow" style={{ fontSize: '10px' }}>
        {kicker}
      </p>
      <h3 className="mt-2 text-[clamp(1.25rem,2vw,1.6rem)] font-semibold tracking-tight text-fd-foreground">
        {title}
      </h3>
      <p className="ministr-body mt-3 text-[14.5px] leading-relaxed">{copy}</p>
    </>
  );
}

/* ---------------------------------------------------------------
   Bento micro-visuals — "tiny instrument panels"
   Each graphic is a mini ministr interface showing the feature happening
   rather than an abstract metaphor. Telegraph: <2 sec to parse.
   --------------------------------------------------------------- */

function PanelShell({
  label,
  children,
  className = '',
}: {
  label: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={
        'overflow-hidden rounded-lg border border-fd-border/50 bg-[color-mix(in_oklch,var(--ministr-surface)_55%,transparent)] ' +
        className
      }
    >
      <div className="flex items-center justify-between border-b border-fd-border/40 px-3 py-1.5">
        <span className="font-mono text-[10px] uppercase tracking-[0.18em] text-[var(--ministr-accent-text)]">
          {label}
        </span>
        <span className="flex gap-1">
          <span className="size-1.5 rounded-full bg-fd-border/70" />
          <span className="size-1.5 rounded-full bg-fd-border/70" />
          <span className="size-1.5 rounded-full bg-fd-border/70" />
        </span>
      </div>
      <div className="p-3">{children}</div>
    </div>
  );
}

/** Session Shadow — a delivered-items ledger. Core telegraph: "ministr
 *  remembers what's been sent, so it won't send it again." */
function TimelineVisual() {
  const rows = [
    { path: 'src/auth.rs#login',        tokens: '142', time: '12:04:18', state: 'sent' as const },
    { path: 'src/auth.rs#logout',       tokens: ' 98', time: '12:04:21', state: 'hit'  as const },
    { path: 'docs/architecture.md#auth', tokens: '310', time: '12:04:35', state: 'sent' as const },
    { path: 'src/middleware.rs#jwt',    tokens: '  —', time: 'warming', state: 'warm' as const },
  ];
  return (
    <PanelShell label="session shadow · 8f2…e1">
      <div className="font-mono text-[11.5px] leading-[1.7]">
        {rows.map((r) => (
          <div
            key={r.path}
            className="flex items-center gap-2 border-b border-fd-border/20 py-[3px] last:border-0"
          >
            {r.state === 'warm' ? (
              <span
                aria-hidden
                className="size-2.5 shrink-0 rounded-full border border-[var(--color-ministr-400)] border-t-transparent animate-spin"
                style={{ animationDuration: '1.6s' }}
              />
            ) : (
              <span
                aria-hidden
                className={
                  'shrink-0 text-[13px] ' +
                  (r.state === 'hit'
                    ? 'text-[var(--color-success)]'
                    : 'text-[var(--ministr-accent-text)]')
                }
              >
                ✓
              </span>
            )}
            <span className="truncate text-fd-foreground">{r.path}</span>
            <span className="ml-auto flex shrink-0 items-center gap-3 tabular-nums text-fd-muted-foreground">
              <span>{r.tokens} tok</span>
              <span className="text-[10.5px]">{r.time}</span>
              {r.state === 'hit' && (
                <span className="rounded bg-[color-mix(in_oklch,var(--color-success)_25%,transparent)] px-1.5 py-px text-[9.5px] font-semibold uppercase tracking-wider text-[var(--color-success)]">
                  hit
                </span>
              )}
            </span>
          </div>
        ))}
      </div>
      <div className="mt-2 flex items-center justify-between text-[10.5px] text-fd-muted-foreground">
        <span>3 sent · 1 warming · next request is free</span>
        <span className="font-mono text-[var(--ministr-accent-text)]">live</span>
      </div>
    </PanelShell>
  );
}

/** Delta Delivery — side-by-side sent-vs-disk with line-level diff markers. */
function DeltaVisual() {
  return (
    <PanelShell label="ministr_read · src/auth.rs#validate">
      <div className="font-mono text-[11.5px] leading-[1.55]">
        <div className="flex items-center justify-between text-[10px] text-fd-muted-foreground">
          <span className="flex items-center gap-1.5">
            <span className="rounded border border-fd-border/60 px-1 py-px">v1 sent</span>
            →
            <span className="rounded border border-[color-mix(in_oklch,var(--color-ministr-400)_45%,transparent)] px-1 py-px text-[var(--ministr-accent-text)]">
              v2 on disk
            </span>
          </span>
          <span className="tabular-nums">+3 / −3</span>
        </div>
        <pre className="mt-2 overflow-hidden rounded bg-[color-mix(in_oklch,var(--ministr-surface-strong)_55%,transparent)] p-2">
          <span className="block text-fd-muted-foreground/50 line-through">
            <span className="select-none pr-2">−</span>fn validate(token: &str) -&gt; bool &#123;
          </span>
          <span className="block text-fd-muted-foreground/50 line-through">
            <span className="select-none pr-2">−</span>  old_check(token)
          </span>
          <span className="block text-fd-muted-foreground/50 line-through">
            <span className="select-none pr-2">−</span>&#125;
          </span>
          <span className="block text-[var(--ministr-accent-text)]">
            <span className="select-none pr-2">+</span>fn validate(tok: &amp;Token) -&gt; Result&lt;Claims&gt; &#123;
          </span>
          <span className="block text-[var(--ministr-accent-text)]">
            <span className="select-none pr-2">+</span>  jwt::decode(tok, &amp;KEY)
          </span>
          <span className="block text-[var(--ministr-accent-text)]">
            <span className="select-none pr-2">+</span>&#125;
          </span>
        </pre>
      </div>
      <div className="mt-2 text-[10.5px] text-fd-muted-foreground">
        shipped <span className="font-mono text-[var(--ministr-accent-text)]">3 lines</span> · skipped{' '}
        <span className="font-mono text-[var(--color-success)]">47 lines</span> (unchanged)
      </div>
    </PanelShell>
  );
}

/** Predictive Prefetch — three-lane flow showing agent → ministr → prefetch queue. */
function PrefetchVisual() {
  const queue = [
    { path: '#logout',  status: 'warm',   progress: 100 },
    { path: '#refresh', status: 'warming', progress: 40 },
    { path: '#revoke',  status: 'queued',  progress: 0 },
  ] as const;
  return (
    <PanelShell label="predictive prefetch">
      <div className="font-mono text-[11px]">
        {/* Trigger row */}
        <div className="flex items-center gap-2 text-fd-foreground">
          <span className="shrink-0 text-[var(--ministr-accent-text)]">➜</span>
          <span className="truncate">ministr_read(src/auth.rs#login)</span>
        </div>
        {/* Arrow to queue */}
        <div className="mt-1.5 flex items-center gap-1 pl-4 text-[10px] text-fd-muted-foreground">
          <span>ministr warms next likely reads</span>
          <span aria-hidden>↓</span>
        </div>
        {/* Queue */}
        <div className="mt-1.5 space-y-1.5 rounded-md border border-fd-border/40 bg-[color-mix(in_oklch,var(--ministr-surface-strong)_40%,transparent)] p-2">
          {queue.map((q) => (
            <div key={q.path} className="flex items-center gap-2">
              <span
                className={
                  'shrink-0 text-[10px] ' +
                  (q.status === 'warm'
                    ? 'text-[var(--color-success)]'
                    : q.status === 'warming'
                    ? 'text-[var(--ministr-accent-text)]'
                    : 'text-fd-muted-foreground/60')
                }
              >
                {q.status === 'warm' ? '●' : q.status === 'warming' ? '◐' : '○'}
              </span>
              <span className="w-[90px] truncate text-fd-foreground">src/auth.rs{q.path}</span>
              <div className="relative h-1 flex-1 overflow-hidden rounded-full bg-[color-mix(in_oklch,var(--ministr-surface-strong)_75%,transparent)]">
                <div
                  className="absolute inset-y-0 left-0 rounded-full bg-[var(--color-ministr-500)]"
                  style={{ width: `${q.progress}%` }}
                />
              </div>
              <span className="w-14 shrink-0 text-right text-[9.5px] uppercase tracking-wider text-fd-muted-foreground">
                {q.status}
              </span>
            </div>
          ))}
        </div>
      </div>
    </PanelShell>
  );
}

/** Budget & Pressure — live token meter with threshold and auto-compress signal. */
function BudgetVisual() {
  return (
    <PanelShell label="context budget">
      <div className="flex items-baseline justify-between">
        <div className="flex items-baseline gap-2">
          <span className="font-mono text-[clamp(1.5rem,2.6vw,2rem)] font-semibold tabular-nums text-fd-foreground">
            84<span className="text-fd-muted-foreground">,237</span>
          </span>
          <span className="text-[11px] text-fd-muted-foreground">/ 100,000 tokens</span>
        </div>
        <span className="flex items-center gap-1.5 rounded-full border border-[color-mix(in_oklch,var(--color-warning)_50%,transparent)] bg-[color-mix(in_oklch,var(--color-warning)_14%,transparent)] px-2 py-0.5 text-[10px] font-mono uppercase tracking-wider text-[var(--color-warning)]">
          <span className="size-1.5 rounded-full bg-[var(--color-warning)] animate-pulse" />
          pressure · elevated
        </span>
      </div>
      <div className="relative mt-3 h-2 overflow-hidden rounded-full bg-[color-mix(in_oklch,var(--ministr-surface-strong)_85%,transparent)]">
        <div className="absolute inset-y-0 left-0 w-[84%] rounded-full bg-[var(--color-warning)]" />
        <div
          aria-label="80% threshold"
          className="absolute inset-y-[-4px] left-[80%] w-px bg-[var(--color-warning)] opacity-90"
        />
      </div>
      <div className="mt-1 flex justify-between text-[9.5px] font-mono uppercase tracking-wider text-fd-muted-foreground">
        <span>0</span>
        <span>threshold · 80%</span>
        <span>100k</span>
      </div>
      <div className="mt-3 grid grid-cols-3 gap-2 text-[11px]">
        <div className="rounded bg-[color-mix(in_oklch,var(--ministr-surface-strong)_50%,transparent)] p-2">
          <div className="text-[9.5px] uppercase tracking-wider text-fd-muted-foreground">auto-compress</div>
          <div className="font-mono text-[var(--color-success)]">−62%</div>
        </div>
        <div className="rounded bg-[color-mix(in_oklch,var(--ministr-surface-strong)_50%,transparent)] p-2">
          <div className="text-[9.5px] uppercase tracking-wider text-fd-muted-foreground">eviction queue</div>
          <div className="font-mono text-[var(--ministr-accent-text)]">3 ready</div>
        </div>
        <div className="rounded bg-[color-mix(in_oklch,var(--ministr-surface-strong)_50%,transparent)] p-2">
          <div className="text-[9.5px] uppercase tracking-wider text-fd-muted-foreground">salience</div>
          <div className="font-mono text-fd-foreground">ranked</div>
        </div>
      </div>
    </PanelShell>
  );
}

/** Coherence — mini file card showing sent version vs live version with change markers. */
function CoherenceVisual() {
  return (
    <PanelShell label="coherence watcher">
      <div className="font-mono text-[11.5px]">
        <div className="mb-2 text-fd-foreground">src/auth.rs</div>
        <div className="flex items-center gap-2 text-[10.5px] text-fd-muted-foreground">
          <span className="w-14 shrink-0 rounded border border-fd-border/50 px-1.5 py-px text-center">sent v1</span>
          <span className="text-[10px]">12:04</span>
          <span className="ml-auto text-fd-muted-foreground/60">now stale</span>
        </div>
        <div className="mt-1.5 flex items-center gap-2 text-[10.5px]">
          <span className="w-14 shrink-0 rounded border border-[color-mix(in_oklch,var(--color-fuchsia-400)_45%,transparent)] bg-[color-mix(in_oklch,var(--color-fuchsia-400)_14%,transparent)] px-1.5 py-px text-center text-[var(--color-fuchsia-400)]">
            disk v2
          </span>
          <span className="text-[10px] text-fd-muted-foreground">12:42</span>
          <span className="ml-auto tabular-nums">
            <span className="text-[var(--color-success)]">+3</span>{' '}
            <span className="text-[var(--color-fuchsia-400)]">−1</span>
          </span>
        </div>
      </div>
      <div className="mt-3 flex items-center gap-2 border-t border-fd-border/30 pt-2 text-[10.5px] text-fd-muted-foreground">
        <span className="relative flex size-2">
          <span className="absolute inset-0 animate-ping rounded-full bg-[var(--color-fuchsia-400)] opacity-60" />
          <span className="relative inline-flex size-2 rounded-full bg-[var(--color-fuchsia-400)]" />
        </span>
        <span>
          ministr flagged <span className="text-fd-foreground">session shadow</span> as stale
        </span>
      </div>
    </PanelShell>
  );
}

/** Hybrid Search — split-score leaderboard showing dense + sparse fused into combined rank. */
function HybridVisual() {
  const results = [
    { path: 'src/limit.rs#token_bucket', dense: 91, sparse: 87, rank: 89 },
    { path: 'src/middleware.rs#throttle', dense: 74, sparse: 82, rank: 78 },
  ];
  return (
    <PanelShell label='ministr_survey("rate limiting")'>
      <div className="space-y-2.5 font-mono text-[10.5px]">
        {results.map((r, i) => (
          <div key={r.path}>
            <div className="flex items-baseline justify-between">
              <span className="truncate text-fd-foreground">{r.path}</span>
              {i === 0 && (
                <span className="ml-2 shrink-0 rounded bg-[color-mix(in_oklch,var(--color-ministr-500)_22%,transparent)] px-1 py-px text-[9px] uppercase tracking-wider text-[var(--ministr-accent-text)]">
                  top
                </span>
              )}
            </div>
            <div className="mt-1 grid grid-cols-[42px_1fr_32px] items-center gap-1.5 text-[9.5px]">
              <span className="uppercase tracking-wider text-[var(--ministr-accent-text)]">dense</span>
              <div className="h-1 overflow-hidden rounded-full bg-[color-mix(in_oklch,var(--ministr-surface-strong)_75%,transparent)]">
                <div
                  className="h-full rounded-full bg-[var(--color-ministr-500)]"
                  style={{ width: `${r.dense}%` }}
                />
              </div>
              <span className="text-right tabular-nums text-fd-muted-foreground">.{r.dense}</span>

              <span className="uppercase tracking-wider text-fd-muted-foreground">sparse</span>
              <div className="h-1 overflow-hidden rounded-full bg-[color-mix(in_oklch,var(--ministr-surface-strong)_75%,transparent)]">
                <div
                  className="h-full rounded-full bg-[color-mix(in_oklch,var(--color-ministr-500)_60%,transparent)]"
                  style={{ width: `${r.sparse}%` }}
                />
              </div>
              <span className="text-right tabular-nums text-fd-muted-foreground">.{r.sparse}</span>

              <span className="uppercase tracking-wider text-fd-foreground">rank</span>
              <div className="h-1 overflow-hidden rounded-full bg-[color-mix(in_oklch,var(--ministr-surface-strong)_75%,transparent)]">
                <div
                  className="h-full rounded-full bg-[var(--color-ministr-600)]"
                  style={{ width: `${r.rank}%` }}
                />
              </div>
              <span className="text-right tabular-nums text-fd-foreground">.{r.rank}</span>
            </div>
          </div>
        ))}
      </div>
      <div className="mt-2 border-t border-fd-border/30 pt-2 text-[10px] text-fd-muted-foreground">
        keyword + semantic, fused at rank-time
      </div>
    </PanelShell>
  );
}
