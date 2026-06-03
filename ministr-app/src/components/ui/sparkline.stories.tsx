import type { Meta, StoryObj } from "@storybook/react-vite";
import { Sparkline } from "./sparkline";

const SERIES = [12, 18, 9, 22, 30, 24, 33, 28, 41, 38, 52, 47];

const meta = {
  title: "UI/Sparkline",
  component: Sparkline,
  args: { data: SERIES, ariaLabel: "Tokens delivered over the last 12 polls" },
} satisfies Meta<typeof Sparkline>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Stepped: Story = {
  render: (args) => (
    <div className="w-60">
      <Sparkline {...args} />
    </div>
  ),
};

export const Smooth: Story = {
  render: (args) => (
    <div className="w-60">
      <Sparkline {...args} smooth height={56} />
    </div>
  ),
};

export const Band: Story = {
  render: () => (
    <div className="w-60">
      <Sparkline
        data={[0, 0, 0, 0, 0, 0]}
        mode="band"
        bandTones={["success", "success", "warning", "warning", "danger", "danger"]}
        height={20}
        ariaLabel="Pressure over time"
      />
    </div>
  ),
};
