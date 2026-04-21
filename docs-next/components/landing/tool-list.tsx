import Link from 'next/link';

type Tool = { name: string; slug: string; description: string };
type Group = { label: string; tools: Tool[] };

const GROUPS: Group[] = [
  {
    label: 'Search & retrieval',
    tools: [
      { name: 'iris_survey', slug: 'survey', description: 'Rank sections matching a natural-language query.' },
      { name: 'iris_read', slug: 'read', description: 'Read a section by id. Returns deltas for changed content and skips what\u2019s already been sent.' },
      { name: 'iris_extract', slug: 'extract', description: 'Pull atomic claims from a section.' },
      { name: 'iris_related', slug: 'related', description: 'Follow claim-to-claim relationships.' },
      { name: 'iris_toc', slug: 'toc', description: 'Structural overview of the indexed corpus.' },
    ],
  },
  {
    label: 'Code navigation',
    tools: [
      { name: 'iris_symbols', slug: 'symbols', description: 'Search code symbols by name, kind, or module.' },
      { name: 'iris_definition', slug: 'definition', description: 'Full source definition of a symbol.' },
      { name: 'iris_references', slug: 'references', description: 'Callers, implementors, importers of a symbol.' },
      { name: 'iris_bridge', slug: 'bridge', description: 'Cross-language FFI / NAPI / PyO3 links.' },
    ],
  },
  {
    label: 'Budget',
    tools: [
      { name: 'iris_budget', slug: 'budget', description: 'Current context budget status.' },
      { name: 'iris_compress', slug: 'compress', description: 'Generate compressed summaries for eviction.' },
      { name: 'iris_evicted', slug: 'evicted', description: 'Signal that content has been evicted.' },
    ],
  },
  {
    label: 'Ingestion',
    tools: [
      { name: 'iris_fetch', slug: 'fetch', description: 'Fetch and index a URL.' },
      { name: 'iris_refresh', slug: 'refresh', description: 'Re-check cached sources for staleness.' },
      { name: 'iris_clone', slug: 'clone', description: 'Clone and index a git repository.' },
    ],
  },
];

export function ToolList() {
  return (
    <div className="space-y-10 [container-type:inline-size]">
      {GROUPS.map((g) => (
        <div key={g.label}>
          <h3 className="iris-eyebrow">{g.label}</h3>
          <dl className="tool-list-dl mt-4 grid gap-x-8 gap-y-3">
            {g.tools.map((t) => (
              <div
                key={t.name}
                className="contents"
              >
                <dt className="font-mono text-[13.5px]">
                  <Link
                    href={`/docs/tools/${t.slug}`}
                    className="text-fd-foreground transition-colors hover:text-[var(--iris-accent-text)]"
                  >
                    {t.name}
                  </Link>
                </dt>
                <dd className="iris-body text-[14px] leading-relaxed">
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
