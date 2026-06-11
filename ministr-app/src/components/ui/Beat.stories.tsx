import type { Meta, StoryObj } from "@storybook/react-vite";
import { Beat } from "./Beat";

const meta = {
  title: "Atoms/Beat",
  component: Beat,
} satisfies Meta<typeof Beat>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Indexing: Story = {
  args: { sentence: "reading your code… 1,204 of 1,482 files" },
  render: (args) => (
    <div className="max-w-md">
      <Beat {...args} />
    </div>
  ),
};
