import type { Meta, StoryObj } from "@storybook/react-vite";
import { AdaptiveSurface } from "./adaptive-surface";
import { Card } from "./card";

const meta = {
  title: "UI/AdaptiveSurface",
  component: AdaptiveSurface,
  args: { children: null },
} satisfies Meta<typeof AdaptiveSurface>;

export default meta;
type Story = StoryObj<typeof meta>;

/**
 * Children adapt to the *container* width (resize the canvas to see the grid
 * go 1 → 2 → 3 columns), not the viewport.
 */
export const ContainerQueryGrid: Story = {
  render: () => (
    <div
      tabIndex={0}
      role="group"
      aria-label="Resizable container-query demo"
      className="h-80 resize-x overflow-auto rounded-lg border border-border-soft"
      style={{ width: 520 }}
    >
      <AdaptiveSurface>
        <div className="grid grid-cols-1 gap-3 p-4 @min-[600px]/surface:grid-cols-2 @min-[900px]/surface:grid-cols-3">
          {Array.from({ length: 6 }, (_, i) => (
            <Card key={i} className="text-sm text-text-muted">
              panel {i + 1}
            </Card>
          ))}
        </div>
      </AdaptiveSurface>
    </div>
  ),
};
