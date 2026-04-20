const ROWS: Array<{ feature: string; grep: string; rag: string; iris: string }> = [
  { feature: 'Retrieval', grep: 'Full-text match', rag: 'Embeddings', iris: 'Embeddings + symbols' },
  { feature: 'Code symbol index', grep: '–', rag: '–', iris: '12 languages' },
  { feature: 'Cross-language bridges', grep: '–', rag: '–', iris: 'yes' },
  { feature: 'Session memory', grep: '–', rag: '–', iris: 'per-corpus shadow' },
  { feature: 'Dedup across turns', grep: '–', rag: '–', iris: 'yes' },
  { feature: 'Delta on change', grep: '–', rag: '–', iris: 'yes' },
  { feature: 'Predictive prefetch', grep: '–', rag: '–', iris: 'yes' },
  { feature: 'Budget awareness', grep: '–', rag: '–', iris: 'tracks + suggests evictions' },
  { feature: 'Agent protocol', grep: 'Shell', rag: 'Custom API', iris: 'MCP (tool-native)' },
  { feature: 'Runs locally', grep: 'yes', rag: 'varies', iris: 'yes' },
];

export function CompareTable() {
  return (
    <div className="mx-auto w-full max-w-4xl px-4 sm:px-6">
      <div className="overflow-x-auto rounded-2xl border border-fd-border bg-fd-card">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-fd-border bg-fd-muted/30">
              <th className="p-3 text-left font-semibold text-xs uppercase tracking-wider text-fd-muted-foreground"></th>
              <th className="p-3 text-center font-mono text-xs text-fd-muted-foreground">
                grep + cat
              </th>
              <th className="p-3 text-center font-mono text-xs text-fd-muted-foreground">
                Vector DB / RAG
              </th>
              <th className="p-3 text-center font-semibold text-[var(--color-iris-500)]">
                iris
              </th>
            </tr>
          </thead>
          <tbody>
            {ROWS.map((row, i) => (
              <tr
                key={row.feature}
                className={i % 2 === 1 ? 'bg-fd-muted/10' : undefined}
              >
                <td className="p-3 font-medium">{row.feature}</td>
                <td className="p-3 text-center text-fd-muted-foreground">{row.grep}</td>
                <td className="p-3 text-center text-fd-muted-foreground">{row.rag}</td>
                <td className="bg-[color-mix(in_srgb,var(--color-iris-500)_8%,transparent)] p-3 text-center font-semibold">
                  {row.iris}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
