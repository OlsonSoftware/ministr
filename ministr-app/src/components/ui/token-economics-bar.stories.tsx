import type { Meta, StoryObj } from "@storybook/react-vite";
import { TokenEconomicsBar } from "./token-economics-bar";

const meta = {
  title: "UI/TokenEconomicsBar",
  component: TokenEconomicsBar,
  args: { deliveredTokens: 184_000, savedTokens: 96_000, liveTokens: 42_000 },
} satisfies Meta<typeof TokenEconomicsBar>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: (args) => (
    <div className="w-[28rem]">
      <TokenEconomicsBar {...args} />
    </div>
  ),
};

export const Empty: Story = {
  render: () => (
    <div className="w-[28rem]">
      <TokenEconomicsBar deliveredTokens={0} savedTokens={0} liveTokens={0} />
    </div>
  ),
};
