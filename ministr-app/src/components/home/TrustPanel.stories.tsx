import type { Meta, StoryObj } from "@storybook/react-vite";
import { TrustPanel } from "./TrustPanel";
import { withTauriMock } from "../../../.storybook/tauri-mock";

const CORPORA = [
  {
    id: "corpus-aaaa",
    display_name: "my-app",
    paths: ["/Users/dev/my-app"],
    status: "idle",
    files_indexed: 214,
    sections_count: 1582,
    active_sessions: 1,
    stack: ["rust", "typescript", "go", "python"],
  },
  {
    id: "corpus-bbbb",
    display_name: "side-project",
    paths: ["/Users/dev/side-project"],
    status: "idle",
    files_indexed: 88,
    sections_count: 412,
    active_sessions: 0,
    stack: ["typescript", "ruby", "elixir", "swift", "kotlin", "scala", "cpp"],
  },
];

const FRESHNESS: Record<string, unknown> = {
  "corpus-aaaa": { current: 1, stale: 0, new: 0, missing: 0, indexing: false },
  "corpus-bbbb": { current: 1, stale: 1, new: 1, missing: 0, indexing: false },
};

const meta = {
  title: "Screens/TrustPanel",
  component: TrustPanel,
  decorators: [
    withTauriMock({
      list_corpora: CORPORA,
      corpus_freshness_summary: (args: Record<string, unknown>) =>
        FRESHNESS[String(args.corpusId)],
    }),
  ],
} satisfies Meta<typeof TrustPanel>;

export default meta;
type Story = StoryObj<typeof meta>;

export const MixedHealth: Story = {
  args: { onOpenProject: () => {}, onAddProject: () => {} },
};

export const Empty: Story = {
  args: { onOpenProject: () => {}, onAddProject: () => {} },
  decorators: [withTauriMock({ list_corpora: [] })],
};

/** A row mid-reindex: the compact Indexing Instrument rides inside the
 *  updating banner (gui-indexing-instrument). */
export const Updating: Story = {
  args: { onOpenProject: () => {}, onAddProject: () => {} },
  decorators: [
    withTauriMock({
      list_corpora: CORPORA,
      corpus_freshness_summary: (args: Record<string, unknown>) =>
        args.corpusId === "corpus-aaaa"
          ? { current: 1, stale: 0, new: 0, missing: 0, indexing: true }
          : FRESHNESS[String(args.corpusId)],
      ingestion_progress: [
        {
          corpus_id: "corpus-aaaa",
          status: 1,
          phase: "embedding",
          files_total: 214,
          files_done: 214,
          sections_done: 1582,
          embeddings_total: 4200,
          embeddings_done: 2604,
          current_file: "src/daemon/indexer.rs",
        },
      ],
    }),
  ],
};
