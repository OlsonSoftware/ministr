import type { Meta, StoryObj } from "@storybook/react-vite";
import { RailRow, RailSection } from "./Rail";
import { TrustMark } from "./TrustMark";

const meta = {
  title: "Atoms/Rail",
  component: RailSection,
} satisfies Meta<typeof RailSection>;

export default meta;
type Story = StoryObj<typeof meta>;

export const ThisProject: Story = {
  args: { label: "this project", children: null },
  render: () => (
    <div className="max-w-xs space-y-4">
      <RailSection label="this project">
        <RailRow label="watching">
          <TrustMark state="ok" />
        </RailRow>
        <RailRow label="updates on save">
          <TrustMark state="ok" />
        </RailRow>
      </RailSection>
      <RailSection label="hidden from your AI">
        <RailRow label="node_modules" />
        <RailRow label=".env" />
      </RailSection>
    </div>
  ),
};
