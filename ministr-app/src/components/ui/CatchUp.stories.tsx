import type { Meta, StoryObj } from "@storybook/react-vite";
import { within } from "storybook/test";
import { CatchUp } from "./CatchUp";
import { ActionChip } from "./ActionChip";
import { withTauriMock } from "../../../.storybook/tauri-mock";

const meta = {
  title: "Atoms/CatchUp",
  component: CatchUp,
} satisfies Meta<typeof CatchUp>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Idle: Story = {
  args: { corpusId: "c1" },
  decorators: [withTauriMock({ trigger_reindex: null })],
};

/** The failure path: a rejecting mock must surface the retry wording. */
export const Failure: Story = {
  args: { corpusId: "c1" },
  decorators: [
    withTauriMock({
      trigger_reindex: () => {
        throw new Error("daemon unreachable");
      },
    }),
  ],
  play: async ({ canvasElement, userEvent }) => {
    const canvas = within(canvasElement);
    await userEvent.click(
      await canvas.findByRole("button", { name: /catch up/i }),
    );
    await canvas.findByRole("button", { name: /couldn.t start — retry/i });
    await canvas.findByText(/is ministr running\?/);
  },
};

/** The busy variant of the underlying chip, storied directly. */
export const BusyChip: Story = {
  args: { corpusId: "c1" },
  render: () => (
    <ActionChip variant="primary" busy>
      Starting…
    </ActionChip>
  ),
};
