import type { Meta, StoryObj } from "@storybook/react-vite";
import { Progress } from "./progress";

const meta = {
  title: "UI/Progress",
  component: Progress,
  args: { value: 62 },
  argTypes: {
    value: { control: { type: "range", min: 0, max: 100 } },
    tone: {
      control: "select",
      options: ["accent", "success", "warning", "danger", "muted"],
    },
    glow: { control: "boolean" },
  },
} satisfies Meta<typeof Progress>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: (args) => (
    <div className="w-80">
      <Progress {...args} />
    </div>
  ),
};

export const LiveGlow: Story = {
  render: () => (
    <div className="flex w-80 flex-col gap-3">
      <Progress value={38} tone="accent" glow />
      <Progress value={72} tone="success" />
      <Progress value={90} tone="warning" />
    </div>
  ),
};
