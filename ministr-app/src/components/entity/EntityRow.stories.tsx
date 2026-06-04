import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { EntityRow } from "./EntityRow";

// NOTE: no `component:` on meta — these are render-based stories, and a
// component-typed meta would force every story to supply `args`.

/**
 * EntityRow — the Cockpit row primitive used inside every EntityPanel view
 * (Symbol / Section / File / Bridge / Corpus / Session). The kind tag is an
 * auto-width chip so it never overlaps the name, however long the tag is.
 * Rendered at the real ~420px drawer width.
 */

const meta = {
  title: "Entity/EntityRow",
  parameters: { layout: "fullscreen" },
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

/** The real EntityPanel content column width. */
function Drawer({ children }: { children: ReactNode }) {
  return (
    <div className="bg-surface" style={{ width: 420 }}>
      <div className="px-5 py-5">
        <div className="overflow-hidden rounded-lg border border-border">
          {children}
        </div>
      </div>
    </div>
  );
}

/** The exact regression: a long tag (FUNCTION) next to a name must not overlap. */
export const KindTags: Story = {
  render: () => (
    <Drawer>
      <EntityRow tag="fn" name="retrieve" subtitle="retrieval::hybrid" onClick={() => {}} />
      <EntityRow
        tag="function"
        name="rerank_candidates"
        subtitle="retrieval::hybrid"
        onClick={() => {}}
      />
      <EntityRow tag="struct" name="IngestPipeline" subtitle="core::ingest" onClick={() => {}} />
      <EntityRow
        tag="namespace"
        name="HybridRetriever"
        subtitle="retrieval"
        onClick={() => {}}
      />
      <EntityRow tag="impl" name="QueryBackend for Local" onClick={() => {}} />
    </Drawer>
  ),
};

export const WithMeta: Story = {
  render: () => (
    <Drawer>
      <EntityRow tag="MD" name="error-handling.md" meta="3§" onClick={() => {}} />
      <EntityRow tag="SECTION" name="Hybrid retrieval" meta="87%" onClick={() => {}} />
    </Drawer>
  ),
};

export const Static: Story = {
  // No onClick → non-interactive (no chevron, no hover lift).
  render: () => (
    <Drawer>
      <EntityRow tag="function" name="retrieve" subtitle="retrieval::hybrid" />
      <EntityRow tag="struct" name="IngestPipeline" subtitle="core::ingest" />
    </Drawer>
  ),
};
