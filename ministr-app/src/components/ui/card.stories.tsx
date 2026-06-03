import type { Meta, StoryObj } from "@storybook/react-vite";
import { Card } from "./card";

const meta = {
  title: "UI/Card",
  component: Card,
  argTypes: {
    hover: { control: "select", options: ["none", "lift", "accent"] },
    sunken: { control: "boolean" },
  },
} satisfies Meta<typeof Card>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  args: {
    children: <div className="text-sm text-text">Tier-1 surface card</div>,
  },
};

export const HoverLift: Story = {
  args: {
    hover: "lift",
    children: <div className="text-sm text-text">Hover me — surface lift</div>,
  },
};

export const HoverAccent: Story = {
  args: {
    hover: "accent",
    children: <div className="text-sm text-text">Hover me — accent ring</div>,
  },
};

export const Sunken: Story = {
  args: {
    sunken: true,
    children: <div className="text-sm text-text-muted">Sunken inset card</div>,
  },
};
