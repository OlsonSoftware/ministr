import type { Meta, StoryObj } from "@storybook/react-vite";
import { Button } from "./button";

const meta = {
  title: "UI/Button",
  component: Button,
  args: { children: "Run query" },
  argTypes: {
    variant: {
      control: "select",
      options: ["default", "ghost", "danger", "outline", "subtle"],
    },
    size: {
      control: "select",
      options: ["sm", "default", "lg", "icon", "icon-sm"],
    },
  },
} satisfies Meta<typeof Button>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const Outline: Story = { args: { variant: "outline" } };
export const Ghost: Story = { args: { variant: "ghost" } };
export const Subtle: Story = { args: { variant: "subtle" } };
export const Danger: Story = {
  args: { variant: "danger", children: "Delete corpus" },
};

export const Sizes: Story = {
  render: (args) => (
    <div className="flex items-center gap-3">
      <Button {...args} size="sm">
        Small
      </Button>
      <Button {...args} size="default">
        Default
      </Button>
      <Button {...args} size="lg">
        Large
      </Button>
    </div>
  ),
};
