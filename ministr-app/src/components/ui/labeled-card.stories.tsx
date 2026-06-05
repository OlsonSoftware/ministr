import type { Meta, StoryObj } from "@storybook/react-vite";
import { Cpu } from "@/components/ui/icons";
import { LabeledCard } from "./labeled-card";
import { LabeledRow } from "./labeled-row";
import { ContentTray } from "./content-tray";

const meta = {
  title: "UI/LabeledCard",
  component: LabeledCard,
  args: { title: "Corpus config", children: null },
} satisfies Meta<typeof LabeledCard>;

export default meta;
type Story = StoryObj<typeof meta>;

export const WithRows: Story = {
  render: () => (
    <div className="w-80">
      <LabeledCard title="Corpus config" icon={Cpu} iconTone="accent">
        <ContentTray compact>
          <LabeledRow label="model" value="jina-code-v2" mono bordered />
          <LabeledRow label="dimension" value="768" mono bordered />
          <LabeledRow label="reranker" value="bge-reranker" mono bordered />
          <LabeledRow label="rerank depth" value="40" mono />
        </ContentTray>
      </LabeledCard>
    </div>
  ),
};
