import type { Meta, StoryObj } from "@storybook/react-vite";
import { ContentTray } from "./content-tray";
import { LabeledRow } from "./labeled-row";

const meta = {
  title: "UI/ContentTray",
  component: ContentTray,
  args: { children: null },
} satisfies Meta<typeof ContentTray>;

export default meta;
type Story = StoryObj<typeof meta>;

export const WithRows: Story = {
  render: () => (
    <div className="w-80">
      <ContentTray>
        <LabeledRow label="model" value="jina-code-v2" mono bordered />
        <LabeledRow label="dimension" value="768" mono bordered />
        <LabeledRow label="reranker" value="bge-reranker" mono />
      </ContentTray>
    </div>
  ),
};

export const Compact: Story = {
  render: () => (
    <div className="w-80">
      <ContentTray compact>
        <p className="text-sm text-text-muted">Recessed tray groups content without a card border.</p>
      </ContentTray>
    </div>
  ),
};
