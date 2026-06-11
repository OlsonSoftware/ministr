import type { Meta, StoryObj } from "@storybook/react-vite";
import { LiveDot } from "./LiveDot";

const meta = {
  title: "Atoms/LiveDot",
  component: LiveDot,
} satisfies Meta<typeof LiveDot>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Presence: Story = {
  args: { label: "your AI is reading LoginForm.tsx" },
};
