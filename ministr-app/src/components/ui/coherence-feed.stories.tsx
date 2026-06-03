import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CoherenceEvent } from "../../lib/types";
import { CoherenceFeed } from "./coherence-feed";

const now = Date.now();
const ev = (over: Partial<CoherenceEvent>): CoherenceEvent =>
  ({
    kind: "modified",
    path: "ministr-core/src/registry.rs",
    affected_sections: ["a", "b"],
    corpus_id: "ministr",
    timestamp_ms: now - 5000,
    ...over,
  }) as unknown as CoherenceEvent;

const EVENTS: CoherenceEvent[] = [
  ev({ kind: "modified", path: "ministr-core/src/registry.rs", affected_sections: ["a", "b", "c"], timestamp_ms: now - 3000 }),
  ev({ kind: "created", path: "ministr-app/src/components/ui/overview.stories.tsx", affected_sections: [], timestamp_ms: now - 18000 }),
  ev({ kind: "removed", path: "ministr-app/src/components/ui/corpus-chip.tsx", affected_sections: ["x"], timestamp_ms: now - 90000 }),
];

const meta = {
  title: "UI/CoherenceFeed",
  component: CoherenceFeed,
  args: { events: EVENTS },
} satisfies Meta<typeof CoherenceFeed>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Populated: Story = {
  render: (args) => (
    <div className="w-[34rem]">
      <CoherenceFeed {...args} />
    </div>
  ),
};

export const Empty: Story = {
  render: () => (
    <div className="w-[34rem]">
      <CoherenceFeed events={[]} />
    </div>
  ),
};
