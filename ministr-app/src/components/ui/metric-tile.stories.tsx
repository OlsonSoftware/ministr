import type { Meta, StoryObj } from "@storybook/react-vite";
import { FileText, Hash, Layers } from "@/components/ui/icons";
import { MetricTile } from "./metric-tile";

const meta = {
  title: "UI/MetricTile",
  component: MetricTile,
  args: { label: "Sections", value: "12,840", icon: Layers, variant: "tile" },
  argTypes: {
    variant: {
      control: "select",
      options: ["tile", "inline", "compact", "cell"],
    },
    tone: {
      control: "select",
      options: [undefined, "success", "warning", "danger", "accent", "muted"],
    },
  },
} satisfies Meta<typeof MetricTile>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Tile: Story = {};

export const Grid: Story = {
  render: () => (
    <div className="grid w-[28rem] grid-cols-3 gap-3">
      <MetricTile icon={FileText} label="Files" value="1,204" />
      <MetricTile icon={Layers} label="Sections" value="12,840" tone="accent" />
      <MetricTile icon={Hash} label="Symbols" value="41,902" />
    </div>
  ),
};
