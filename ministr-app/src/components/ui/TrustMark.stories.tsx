import type { Meta, StoryObj } from "@storybook/react-vite";
import { TrustMark } from "./TrustMark";
import { TRUST, type TrustState } from "./trust";

const meta = {
  title: "Atoms/TrustMark",
  component: TrustMark,
} satisfies Meta<typeof TrustMark>;

export default meta;
type Story = StoryObj<typeof meta>;

export const AllStates: Story = {
  args: { state: "ok" },
  render: () => (
    <div className="space-y-2">
      {(Object.keys(TRUST) as TrustState[]).map((s) => (
        <div key={s} className="flex items-center gap-3 text-sm text-ink">
          <TrustMark state={s} />
          <span>{TRUST[s].word}</span>
        </div>
      ))}
    </div>
  ),
};
