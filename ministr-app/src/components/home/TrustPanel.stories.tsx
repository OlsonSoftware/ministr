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
  },
  {
    id: "corpus-bbbb",
    display_name: "side-project",
    paths: ["/Users/dev/side-project"],
    status: "idle",
    files_indexed: 88,
    sections_count: 412,
    active_sessions: 0,
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
  args: { onOpenProject: () => {} },
};

export const Empty: Story = {
  args: { onOpenProject: () => {} },
  decorators: [withTauriMock({ list_corpora: [] })],
};
