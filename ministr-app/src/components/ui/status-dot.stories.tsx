import type { Meta, StoryObj } from "@storybook/react-vite";
import { StatusDot } from "./status-dot";

const TONES = ["success", "warning", "danger", "accent", "muted"] as const;

const meta = {
  title: "UI/StatusDot",
  component: StatusDot,
  argTypes: {
    tone: { control: "select", options: TONES },
    pulse: { control: "inline-radio", options: ["off", "live"] },
    size: { control: "inline-radio", options: ["sm", "md"] },
  },
} satisfies Meta<typeof StatusDot>;

export default meta;
type Story = StoryObj<typeof meta>;

export const AllTones: Story = {
  render: () => (
    <div className="flex items-center gap-4">
      {TONES.map((t) => (
        <span
          key={t}
          className="inline-flex items-center gap-1.5 text-xs text-text-muted"
        >
          <StatusDot tone={t} /> {t}
        </span>
      ))}
    </div>
  ),
};

export const LivePulse: Story = {
  args: { tone: "accent", pulse: "live", size: "md" },
};
