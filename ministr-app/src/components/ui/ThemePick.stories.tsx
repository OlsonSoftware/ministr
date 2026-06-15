import type { Meta, StoryObj } from "@storybook/react-vite";
import { within, userEvent, expect } from "storybook/test";
import { ThemePick } from "./ThemePick";

/**
 * A demoted, icon-only appearance affordance (C4). Render-only for the
 * collapsed state; the Open story drives the popover so axe also checks
 * it. The class-flip behavior stays pinned by lib/theme's resolveDark
 * unit tests, not a play function.
 */
const meta = {
  title: "Atoms/ThemePick",
  component: ThemePick,
} satisfies Meta<typeof ThemePick>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Collapsed — the single quiet appearance icon (the only chrome on Home). */
export const Default: Story = {};

/** Opened — the System/Light/Dark choices in the popover. */
export const Open: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: "Appearance" }));
    await expect(canvas.getByRole("radiogroup")).toBeInTheDocument();
    await expect(canvas.getByRole("radio", { name: /Match my Mac/ })).toBeInTheDocument();
  },
};
