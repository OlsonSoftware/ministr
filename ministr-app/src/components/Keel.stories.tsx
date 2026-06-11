import type { Meta, StoryObj } from "@storybook/react-vite";
import { Keel } from "./Keel";

/**
 * Seed story — proves the emptied Storybook harness end-to-end (render +
 * axe in light AND dark via the two Vitest browser projects).
 */
const meta = {
  title: "Seed/Keel",
  component: Keel,
} satisfies Meta<typeof Keel>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  args: {
    title: "ministr",
    line: "rebuilding from the keel — UX-BLUEPRINT v4",
  },
};
