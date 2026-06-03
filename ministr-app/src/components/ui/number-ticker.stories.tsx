import type { Meta, StoryObj } from "@storybook/react-vite";
import { NumberTicker } from "./number-ticker";

const meta = {
  title: "UI/NumberTicker",
  component: NumberTicker,
  args: { value: 184_002 },
} satisfies Meta<typeof NumberTicker>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: (args) => (
    <span className="text-2xl text-text">
      <NumberTicker {...args} />
    </span>
  ),
};

export const Tokens: Story = {
  render: () => (
    <div className="flex items-baseline gap-2 text-text">
      <span className="text-3xl font-semibold">
        <NumberTicker value={96_400} flashOnChange />
      </span>
      <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
        tokens saved
      </span>
    </div>
  ),
};
