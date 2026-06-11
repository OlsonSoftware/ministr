import type { Meta, StoryObj } from "@storybook/react-vite";
import { ActionChip } from "./ActionChip";

const meta = {
  title: "Atoms/ActionChip",
  component: ActionChip,
} satisfies Meta<typeof ActionChip>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Variants: Story = {
  render: () => (
    <div className="flex items-center gap-3">
      <ActionChip variant="primary">Catch up · ~40s</ActionChip>
      <ActionChip>Update now</ActionChip>
    </div>
  ),
};
