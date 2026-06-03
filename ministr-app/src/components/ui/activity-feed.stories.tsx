import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ActivityEvent } from "../../lib/types";
import { ActivityFeed } from "./activity-feed";

const now = Date.now();
const ev = (over: Partial<ActivityEvent>): ActivityEvent =>
  ({
    tool: "ministr_survey",
    summary: "hybrid retrieval over ministr-core",
    corpus_id: "ministr",
    session_id: "a1b2c3d4",
    timestamp_ms: now - 4000,
    tokens_delta: 1840,
    cache_hit: false,
    ...over,
  }) as unknown as ActivityEvent;

const EVENTS: ActivityEvent[] = [
  ev({ tool: "ministr_survey", summary: "hybrid retrieval over ministr-core", timestamp_ms: now - 2000, tokens_delta: 1840 }),
  ev({ tool: "ministr_read", summary: "registry.rs#get_or_lazy_load", timestamp_ms: now - 9000, cache_hit: true }),
  ev({ tool: "ministr_symbols", summary: "CorpusRegistry", timestamp_ms: now - 21000, tokens_delta: 420 }),
  ev({ tool: "ministr_references", summary: "DaemonClient::reindex_corpus", timestamp_ms: now - 64000, tokens_delta: 980 }),
  ev({ tool: "ministr_bridge", summary: "tauri command map", timestamp_ms: now - 180000, cache_hit: true }),
];

const meta = {
  title: "UI/ActivityFeed",
  component: ActivityFeed,
  args: { events: EVENTS },
} satisfies Meta<typeof ActivityFeed>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Populated: Story = {
  render: (args) => (
    <div className="w-[34rem]">
      <ActivityFeed {...args} />
    </div>
  ),
};

export const Empty: Story = {
  render: () => (
    <div className="w-[34rem]">
      <ActivityFeed events={[]} />
    </div>
  ),
};
