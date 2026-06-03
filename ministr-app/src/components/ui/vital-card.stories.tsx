import type { Meta, StoryObj } from "@storybook/react-vite";
import { VitalCard } from "./vital-card";
import { Sparkline } from "./sparkline";
import { Badge } from "./badge";

const meta = {
  title: "UI/VitalCard",
  component: VitalCard,
  args: { title: "Retrieval", subtitle: "last 12 polls", children: <span /> },
} satisfies Meta<typeof VitalCard>;

export default meta;
type Story = StoryObj<typeof meta>;

export const WithChart: Story = {
  render: (args) => (
    <div className="w-72">
      <VitalCard {...args} right={<Badge variant="success" dot>live</Badge>}>
        <Sparkline
          data={[12, 18, 9, 22, 30, 24, 33, 28, 41, 38, 52, 47]}
          smooth
          height={64}
          ariaLabel="Tokens delivered over time"
        />
      </VitalCard>
    </div>
  ),
};

export const Empty: Story = {
  render: () => (
    <div className="w-72">
      <VitalCard title="Sessions" empty emptyLabel="No active sessions">
        <span />
      </VitalCard>
    </div>
  ),
};
