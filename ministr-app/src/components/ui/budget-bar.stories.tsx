import type { Meta, StoryObj } from "@storybook/react-vite";
import { BudgetBar } from "./budget-bar";

const meta = {
  title: "UI/BudgetBar",
  component: BudgetBar,
  args: { utilization: 0.62, size: "hero" },
  argTypes: {
    utilization: { control: { type: "range", min: 0, max: 1, step: 0.01 } },
    size: { control: "inline-radio", options: ["hero", "card"] },
    showValue: { control: "boolean" },
  },
} satisfies Meta<typeof BudgetBar>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Hero: Story = {
  render: (args) => (
    <div className="w-96">
      <BudgetBar {...args} />
    </div>
  ),
};

export const Scale: Story = {
  render: () => (
    <div className="flex w-96 flex-col gap-3">
      <BudgetBar utilization={0.25} size="hero" showValue />
      <BudgetBar utilization={0.6} size="hero" showValue />
      <BudgetBar utilization={0.85} size="hero" showValue />
      <BudgetBar utilization={0.97} size="hero" showValue />
    </div>
  ),
};
