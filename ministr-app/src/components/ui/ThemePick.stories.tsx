import type { Meta, StoryObj } from "@storybook/react-vite";
import { ThemePick } from "./ThemePick";

/**
 * Render-only on purpose: the dual-theme vitest harness owns
 * documentElement.class, so the class-flip behavior is pinned by
 * lib/theme's resolveDark unit tests, not a play function.
 */
const meta = {
  title: "Atoms/ThemePick",
  component: ThemePick,
} satisfies Meta<typeof ThemePick>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
