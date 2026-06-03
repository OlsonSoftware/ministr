import type { Meta, StoryObj } from "@storybook/react-vite";
import { BudgetRing } from "./budget-ring";

const meta = {
  title: "UI/BudgetRing",
  component: BudgetRing,
  args: { utilization: 0.62, pressure: "medium" },
  argTypes: {
    utilization: { control: { type: "range", min: 0, max: 1, step: 0.01 } },
    warm: { control: { type: "range", min: 0, max: 1, step: 0.01 } },
    pressure: {
      control: "select",
      options: ["none", "low", "medium", "high", "critical"],
    },
  },
} satisfies Meta<typeof BudgetRing>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: (args) => (
    <BudgetRing {...args}>
      <span className="font-mono text-lg font-semibold tabular-nums text-text">
        {Math.round(args.utilization * 100)}%
      </span>
      <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
        budget
      </span>
    </BudgetRing>
  ),
};

export const PressureScale: Story = {
  render: () => (
    <div className="flex items-center gap-6">
      {(["low", "medium", "high", "critical"] as const).map((p, i) => (
        <BudgetRing key={p} utilization={0.35 + i * 0.2} warm={0.2} pressure={p}>
          <span className="font-mono text-base font-semibold tabular-nums text-text">
            {Math.round((0.35 + i * 0.2) * 100)}%
          </span>
        </BudgetRing>
      ))}
    </div>
  ),
};
