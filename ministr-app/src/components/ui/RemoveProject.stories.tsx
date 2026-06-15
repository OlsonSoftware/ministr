import type { Meta, StoryObj } from "@storybook/react-vite";
import { within, expect } from "storybook/test";
import { RemoveProject } from "./RemoveProject";
import { withTauriMock } from "../../../.storybook/tauri-mock";

const meta = {
  title: "Atoms/RemoveProject",
  component: RemoveProject,
  args: { corpusId: "corpus-bbbb", displayName: "side-project", onRemoved: () => {} },
  decorators: [withTauriMock({ remove_project: undefined })],
} satisfies Meta<typeof RemoveProject>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Resting: a quiet, non-emphasised "Remove project" trigger. */
export const Idle: Story = {};

/** After the trigger: an explicit consequence + a named "Forget" action
 *  beside an easy Cancel (friction-for-safety). */
export const Confirming: Story = {
  play: async ({ canvasElement, userEvent }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole("button", { name: /remove side-project/i }));
    await canvas.findByText(/Forget side-project\?/);
    await expect(
      canvas.getByRole("button", { name: /forget side-project/i }),
    ).toBeVisible();
    await expect(canvas.getByRole("button", { name: /cancel/i })).toBeVisible();
  },
};
