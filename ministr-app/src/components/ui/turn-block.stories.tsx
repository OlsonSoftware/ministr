import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CorpusInfo, SessionDetail } from "../../lib/types";
import { TurnBlock } from "./turn-block";

const CORPORA = [
  {
    id: "ministr",
    display_name: "ministr",
    paths: ["/Users/alrik/Code/ministr"],
  },
] as unknown as CorpusInfo[];

const session = (over: Partial<SessionDetail>): SessionDetail =>
  ({
    session_id: "a1b2c3d4e5f6",
    current_turn: 14,
    utilization: 0.62,
    tokens_used: 124_000,
    total_tokens_saved: 88_000,
    dedup_hits: 1240,
    corpus_id: "ministr",
    client_name: "claude-code",
    ...over,
  }) as unknown as SessionDetail;

const meta = {
  title: "UI/TurnBlock",
  component: TurnBlock,
  args: { session: session({}), corpora: CORPORA, onClick: () => {} },
} satisfies Meta<typeof TurnBlock>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Healthy: Story = {
  render: (args) => (
    <div className="w-80">
      <TurnBlock {...args} />
    </div>
  ),
};

export const Critical: Story = {
  render: () => (
    <div className="w-80">
      <TurnBlock
        session={session({ utilization: 0.97 })}
        corpora={CORPORA}
        fresh
        onClick={() => {}}
      />
    </div>
  ),
};
