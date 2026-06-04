import type { Meta, StoryObj } from "@storybook/react-vite";
import { ConnectingState } from "./ConnectingState";

/**
 * The boot screen — shown on every cold launch until the daemon status
 * resolves. The flattest, most-seen first impression in the app, rebuilt as a
 * command-deck "starting up" hero. Two states: connecting (live) and the
 * unreachable-daemon error.
 */
const meta = {
  title: "Shell/ConnectingState",
  component: ConnectingState,
  parameters: { layout: "fullscreen" },
  decorators: [
    (Story) => (
      <div className="h-screen bg-bg">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ConnectingState>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Connecting: Story = {
  args: { error: null },
};

export const Error: Story = {
  args: {
    error:
      "Couldn’t reach the ministr daemon. It may still be starting — retrying automatically.",
  },
};
