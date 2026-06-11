import type { Meta, StoryObj } from "@storybook/react-vite";
import { Brand } from "./Brand";

const meta = {
  title: "Atoms/Brand",
  component: Brand,
} satisfies Meta<typeof Brand>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Sizes: Story = {
  render: () => (
    <div className="space-y-4">
      <Brand />
      <Brand size="lg" />
    </div>
  ),
};
