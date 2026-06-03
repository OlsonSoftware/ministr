import type { Meta, StoryObj } from "@storybook/react-vite";
import { CodeExcerpt } from "./code-excerpt";

/**
 * CodeExcerpt — reusable Shiki-highlighted snippet. Grammar is inferred from
 * the filename; it preserves (de-dented) formatting, clamps to `maxLines`,
 * and renders on a transparent background. Review in light + dark.
 */
const meta = {
  title: "UI/CodeExcerpt",
  component: CodeExcerpt,
  parameters: { layout: "centered" },
  decorators: [
    (Story) => (
      <div className="w-[460px] rounded-lg border border-border bg-surface p-3">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof CodeExcerpt>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Rust: Story = {
  args: {
    filename: "ministr-core/src/retrieve.rs",
    code: `pub fn retrieve(query: &Query) -> Vec<Section> {
    // hybrid: dense + sparse, then rerank
    let dense = self.dense.search(query, 64);
    let sparse = self.sparse.search(query, 64);
    rerank(merge(dense, sparse))
}`,
  },
};

export const TypeScript: Story = {
  args: {
    filename: "src/lib/corpus.ts",
    code: `export function mergeCorpora(a: Corpus[], b: Corpus[]): Corpus[] {
  const byId = new Map(a.map((c) => [c.id, c]));
  for (const c of b) byId.set(c.id, { ...byId.get(c.id), ...c });
  return [...byId.values()];
}`,
  },
};

export const Python: Story = {
  args: {
    filename: "scripts/embed.py",
    code: `def embed(texts: list[str]) -> np.ndarray:
    batches = chunk(texts, size=32)
    return np.concatenate([model.encode(b) for b in batches])`,
  },
};

/** Clamps to 3 lines and appends an ellipsis line. */
export const Clamped: Story = {
  args: {
    filename: "ministr-core/src/ingest.rs",
    maxLines: 3,
    code: `pub struct IngestPipeline {
    embedder: Embedder,
    store: Store,
    parser: Parser,
    governor: Governor,
}`,
  },
};

/** Unknown grammar → plain-text fallback (no colour, formatting intact). */
export const PlainFallback: Story = {
  args: {
    filename: "notes.unknownext",
    code: `just some
  indented plain
text with no grammar`,
  },
};
