export function ObservatoryPreview() {
  return (
    <div
      role="img"
      aria-label="Preview of the iris desktop observatory — macOS window showing three corpora in the sidebar, two live sessions, a query playground with ranked results, and an indexing progress bar."
      className="mx-auto w-full max-w-5xl rounded-2xl border border-fd-border bg-fd-card shadow-lg overflow-hidden"
    >
      {/* Chrome */}
      <div className="flex items-center gap-2 border-b border-fd-border bg-fd-muted/20 px-3 py-2">
        <span className="size-3 rounded-full bg-[var(--color-traffic-close)]" />
        <span className="size-3 rounded-full bg-[var(--color-traffic-min)]" />
        <span className="size-3 rounded-full bg-[var(--color-traffic-max)]" />
        <span className="ml-3 text-xs font-mono text-fd-muted-foreground">iris — observatory</span>
        <span className="ml-auto flex items-center gap-1.5 text-[10px] font-mono text-fd-muted-foreground">
          <span className="size-1.5 rounded-full bg-[var(--color-success)]" />
          daemon connected
        </span>
      </div>

      {/* Body */}
      <div className="grid grid-cols-[180px_1fr] sm:grid-cols-[200px_1fr]">
        {/* Sidebar */}
        <aside className="border-r border-fd-border bg-fd-muted/10 p-3 text-xs">
          <div className="mb-1.5 text-[9px] font-semibold uppercase tracking-wider text-fd-muted-foreground">
            Corpora · 3
          </div>
          <ul className="mb-4 space-y-0.5">
            <SidebarRow name="iris-rs" meta="4128 docs" active />
            <SidebarRow name="docs/" meta="312 docs" />
            <SidebarRow name="research-notes" meta="57 docs" />
          </ul>
          <div className="mb-1.5 text-[9px] font-semibold uppercase tracking-wider text-fd-muted-foreground">
            Sessions · 2 live
          </div>
          <ul className="space-y-0.5">
            <SidebarRow name="claude-code · main" meta="42%" />
            <SidebarRow name="cursor · refactor" meta="18%" />
          </ul>
        </aside>

        {/* Main */}
        <div className="space-y-3 p-4">
          {/* Query playground */}
          <div className="rounded-lg border border-fd-border bg-fd-background p-3">
            <div className="mb-2 flex items-center justify-between text-[11px] font-mono">
              <span className="font-semibold">Query playground</span>
              <span className="text-fd-muted-foreground">
                iris_survey · 5 hits · 42 ms
              </span>
            </div>
            <div className="mb-2 rounded-md bg-fd-muted/30 px-2 py-1.5 font-mono text-xs">
              authentication middleware
            </div>
            <div className="space-y-2">
              <ResultRow
                path="src/auth.rs › login"
                score="0.91"
                snippet="Validates JWT tokens using RS256 and calls"
                snippetCode="validate_token"
                snippetTrail="…"
              />
              <ResultRow
                path="src/auth.rs › logout"
                score="0.87"
                snippet="Revokes the session cookie and blacklists the refresh token until…"
              />
            </div>
          </div>

          {/* Indexing progress */}
          <div className="rounded-lg border border-fd-border bg-fd-background p-3">
            <div className="mb-2 flex items-center justify-between text-[11px] font-mono">
              <span className="font-semibold">Indexing · iris-rs</span>
              <span className="text-fd-muted-foreground">2812 / 4128 sections</span>
            </div>
            <div
              className="h-1.5 overflow-hidden rounded-full bg-fd-muted/40"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={68}
              aria-label="Indexing progress"
            >
              <span
                className="block h-full bg-[var(--color-iris-500)] transition-all"
                style={{ width: '68%' }}
              />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function SidebarRow({
  name,
  meta,
  active = false,
}: {
  name: string;
  meta: string;
  active?: boolean;
}) {
  return (
    <li
      className={
        'flex items-center justify-between gap-2 rounded px-1.5 py-1 ' +
        (active
          ? 'bg-[color-mix(in_srgb,var(--color-iris-500)_12%,transparent)] text-[var(--color-iris-500)]'
          : '')
      }
    >
      <span className="truncate font-mono text-[11px] font-medium">{name}</span>
      <span className="shrink-0 font-mono text-[10px] text-fd-muted-foreground">{meta}</span>
    </li>
  );
}

function ResultRow({
  path,
  score,
  snippet,
  snippetCode,
  snippetTrail,
}: {
  path: string;
  score: string;
  snippet: string;
  snippetCode?: string;
  snippetTrail?: string;
}) {
  return (
    <div className="rounded border border-fd-border/60 bg-fd-muted/10 p-2">
      <div className="mb-1 flex items-center justify-between gap-2 text-[11px]">
        <span className="truncate font-mono text-fd-muted-foreground">{path}</span>
        <span className="shrink-0 font-mono text-[10px] text-[var(--color-iris-500)]">
          {score}
        </span>
      </div>
      <p className="text-[11px] leading-snug text-fd-muted-foreground">
        {snippet}
        {snippetCode && <code className="font-mono text-fd-foreground"> {snippetCode}</code>}
        {snippetTrail}
      </p>
    </div>
  );
}
