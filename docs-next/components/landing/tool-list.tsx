import Link from 'next/link';

type Tool = { name: string; slug: string; description: string };
type Group = { label: string; tools: Tool[] };

const GROUPS: Group[] = [
  {
    label: 'Search & retrieval',
    tools: [
      { name: 'ministr_survey', slug: 'survey', description: 'Rank sections matching a natural-language query.' },
      { name: 'ministr_read', slug: 'read', description: 'Read a section by id. Returns deltas for changed content and skips what\u2019s already been sent.' },
      { name: 'ministr_extract', slug: 'extract', description: 'Pull atomic claims from a section.' },
      { name: 'ministr_related', slug: 'related', description: 'Follow claim-to-claim relationships.' },
      { name: 'ministr_toc', slug: 'toc', description: 'Structural overview of the indexed corpus.' },
    ],
  },
  {
    label: 'Code navigation',
    tools: [
      { name: 'ministr_symbols', slug: 'symbols', description: 'Search code symbols by name, kind, or module.' },
      { name: 'ministr_definition', slug: 'definition', description: 'Full source definition of a symbol.' },
      { name: 'ministr_references', slug: 'references', description: 'Callers, implementors, importers of a symbol.' },
      { name: 'ministr_bridge', slug: 'bridge', description: 'Cross-language FFI / NAPI / PyO3 links.' },
    ],
  },
  {
    label: 'Session efficiency',
    tools: [
      { name: 'ministr_usage', slug: 'usage', description: 'Advisory token-usage estimate (internal accounting).' },
      { name: 'ministr_compress', slug: 'compress', description: 'Generate compact summaries of sections.' },
      { name: 'ministr_dropped', slug: 'dropped', description: 'Signal that content was dropped from context.' },
    ],
  },
  {
    label: 'Ingestion',
    tools: [
      { name: 'ministr_fetch', slug: 'fetch', description: 'Fetch and index a URL.' },
      { name: 'ministr_refresh', slug: 'refresh', description: 'Re-check cached sources for staleness.' },
      { name: 'ministr_clone', slug: 'clone', description: 'Clone and index a git repository.' },
    ],
  },
];

export function ToolList() {
  return (
    <div className="space-y-10 [container-type:inline-size]">
      {GROUPS.map((g) => (
        <div key={g.label}>
          <h3 className="ministr-eyebrow">{g.label}</h3>
          <dl className="tool-list-dl mt-4 grid gap-x-8 gap-y-3">
            {g.tools.map((t) => (
              <div
                key={t.name}
                className="contents"
              >
                <dt className="font-mono text-[13.5px]">
                  <Link
                    href={`/docs/tools/${t.slug}`}
                    className="text-fd-foreground transition-colors hover:text-[var(--ministr-accent-text)]"
                  >
                    {t.name}
                  </Link>
                </dt>
                <dd className="ministr-body text-[14px] leading-relaxed">
                  {t.description}
                </dd>
              </div>
            ))}
          </dl>
        </div>
      ))}
    </div>
  );
}
