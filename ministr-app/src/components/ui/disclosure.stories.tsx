import type { Meta, StoryObj } from "@storybook/react-vite";
import { Disclosure } from "./disclosure";

const meta = {
  title: "UI/Disclosure",
  component: Disclosure,
  args: { title: "Retrieval settings", defaultOpen: true, children: null },
} satisfies Meta<typeof Disclosure>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Open: Story = {
  render: (args) => (
    <div className="w-96">
      <Disclosure {...args} chapter={3} meta="4 keys">
        <div className="px-4 py-3 text-sm text-text-muted">
          Hybrid retrieval, reranker depth, and Matryoshka dimension live here.
        </div>
      </Disclosure>
    </div>
  ),
};

export const Closed: Story = {
  render: () => (
    <div className="w-96">
      <Disclosure title="Advanced" defaultOpen={false} meta="optional">
        <div className="px-4 py-3 text-sm text-text-muted">Hidden until opened.</div>
      </Disclosure>
    </div>
  ),
};
