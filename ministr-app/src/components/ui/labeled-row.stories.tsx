import type { Meta, StoryObj } from "@storybook/react-vite";
import { LabeledRow } from "./labeled-row";

const meta = {
  title: "UI/LabeledRow",
  component: LabeledRow,
  args: { label: "model", value: "jina-code-v2", mono: true },
} satisfies Meta<typeof LabeledRow>;

export default meta;
type Story = StoryObj<typeof meta>;

export const List: Story = {
  render: () => (
    <div className="w-80">
      <LabeledRow label="model" value="jina-code-v2" mono bordered />
      <LabeledRow label="dimension" value="768" mono bordered />
      <LabeledRow label="reranker" value="bge-reranker" mono bordered />
      <LabeledRow label="rerank depth" value="40" mono />
    </div>
  ),
};
