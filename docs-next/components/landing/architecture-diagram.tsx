import Link from 'next/link';

export function ArchitectureDiagram() {
  return (
    <div className="mx-auto w-full max-w-4xl px-4 sm:px-6">
      <div className="rounded-2xl border border-fd-border bg-fd-card p-8 sm:p-12">
        <div className="grid items-center gap-6 md:grid-cols-[1fr_auto_1fr_auto_1fr]">
          <Box
            label="Your MCP client"
            detail="Claude Code · Cursor · any MCP client"
          />
          <Arrow />
          <Box
            label="iris"
            detail="MCP server · runs locally"
            accent
          />
          <Arrow />
          <Box
            label="Your corpus"
            detail="Source files · local index"
          />
        </div>
      </div>
      <p className="mt-3 text-center text-sm text-fd-muted-foreground">
        <Link
          href="/docs/architecture"
          className="underline-offset-4 hover:underline"
        >
          See the full architecture →
        </Link>
      </p>
    </div>
  );
}

function Box({
  label,
  detail,
  accent = false,
}: {
  label: string;
  detail: string;
  accent?: boolean;
}) {
  return (
    <div
      className={
        'rounded-xl border p-4 text-center ' +
        (accent
          ? 'border-[color-mix(in_srgb,var(--color-iris-400)_50%,transparent)] bg-[color-mix(in_srgb,var(--color-iris-500)_8%,transparent)]'
          : 'border-fd-border bg-fd-background')
      }
    >
      <div className="text-sm font-semibold">{label}</div>
      <div className="mt-1 text-xs text-fd-muted-foreground">{detail}</div>
    </div>
  );
}

function Arrow() {
  return (
    <div className="flex justify-center text-fd-muted-foreground">
      <svg
        width="24"
        height="24"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        aria-hidden
      >
        <path d="M5 12h14M13 5l7 7-7 7" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    </div>
  );
}
