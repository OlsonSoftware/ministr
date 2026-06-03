import type { Meta, StoryObj } from "@storybook/react-vite";
import { Badge } from "./badge";

const meta = {
  title: "UI/Badge",
  component: Badge,
  args: { children: "indexed", variant: "default" },
  argTypes: {
    variant: {
      control: "select",
      options: ["default", "success", "warning", "danger", "muted"],
    },
    dot: { control: "boolean" },
  },
} satisfies Meta<typeof Badge>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const WithDot: Story = { args: { dot: true } };

export const AllVariants: Story = {
  render: () => (
    <div className="flex flex-wrap items-center gap-2">
      <Badge variant="default" dot>
        default
      </Badge>
      <Badge variant="success" dot>
        ready
      </Badge>
      <Badge variant="warning" dot>
        warming
      </Badge>
      <Badge variant="danger" dot>
        error
      </Badge>
      <Badge variant="muted">muted</Badge>
    </div>
  ),
};
