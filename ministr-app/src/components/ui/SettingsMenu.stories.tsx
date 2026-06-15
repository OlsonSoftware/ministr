import type { Meta, StoryObj } from "@storybook/react-vite";
import { within } from "storybook/test";
import { SettingsMenu } from "./SettingsMenu";
import { withTauriMock } from "../../../.storybook/tauri-mock";

const STATUS = {
  version: "0.7.0",
  uptime_secs: 12_240, // 3h 24m
  memory_mb: 184.6,
  model: "bge-small-en-v1.5",
  model_dimension: 384,
  corpora: [],
  log_path: "/Users/dev/Library/Logs/ministr/daemon.log",
  total_sessions: 7,
  autostart_enabled: true,
};

const meta = {
  title: "Atoms/SettingsMenu",
  component: SettingsMenu,
  // Header-anchored popover: pin it to the top-right like the real mount so
  // the panel opens downward in frame.
  decorators: [
    (Story) => (
      <div className="flex min-h-64 justify-end p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SettingsMenu>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Closed — the labeled "Settings" trigger as it sits in the Home header. */
export const Trigger: Story = {
  decorators: [withTauriMock({ daemon_status: STATUS })],
};

/** Open — appearance choices, the live version + daemon status, and a
 *  Documentation link, all in one place. */
export const Open: Story = {
  decorators: [withTauriMock({ daemon_status: STATUS })],
  play: async ({ canvasElement, userEvent }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: /settings/i }));
    await canvas.findByText(/Version 0\.7\.0/);
    await canvas.findByText(/up 3h 24m/);
    await canvas.findByRole("button", { name: /documentation/i });
  },
};

/** Open, daemon down — the About section says so honestly (no fake restart),
 *  pointing the user at a relaunch. */
export const DaemonDown: Story = {
  decorators: [
    withTauriMock({
      daemon_status: () => {
        throw new Error("connection refused");
      },
    }),
  ],
  play: async ({ canvasElement, userEvent }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: /settings/i }));
    await canvas.findByText(/ministr isn’t running/);
  },
};
