import type { Meta, StoryObj } from "@storybook/react-vite";
import { within } from "storybook/test";
import { TrustPanel } from "./TrustPanel";
import { withTauriMock } from "../../../.storybook/tauri-mock";

/**
 * Connection states (gui-rw-daemon-down-states): boot while the first
 * fetch is in flight, unreachable when nothing ever loads, degraded
 * note when last-good data is on screen but polls start failing.
 */
const meta = {
  title: "Screens/TrustPanel/Connection",
  component: TrustPanel,
} satisfies Meta<typeof TrustPanel>;

export default meta;
type Story = StoryObj<typeof meta>;

/** First fetch never resolves → the connecting beat. */
export const Connecting: Story = {
  args: { onOpenProject: () => {} },
  decorators: [
    withTauriMock({
      list_corpora: () => new Promise(() => {}),
    }),
  ],
  play: async ({ canvasElement }) => {
    await within(canvasElement).findByText(/connecting to ministr…/);
  },
};

/** Every fetch fails, nothing to show → the unreachable banner. */
export const Unreachable: Story = {
  args: { onOpenProject: () => {} },
  decorators: [
    withTauriMock({
      list_corpora: () => {
        throw new Error("connection refused");
      },
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await canvas.findByText(/ministr isn’t running on this Mac/);
    await canvas.findByText(/reconnects automatically/);
  },
};

/** Data loaded once, then polls fail → last-good rows + honest note. */
export const ConnectionLost: Story = {
  args: { onOpenProject: () => {} },
  decorators: [
    (() => {
      let calls = 0;
      return withTauriMock({
        list_corpora: () => {
          calls += 1;
          if (calls > 1) throw new Error("connection refused");
          return [
            {
              id: "corpus-aaaa",
              display_name: "my-app",
              paths: ["/u/me/my-app"],
              files_indexed: 12,
              active_sessions: 0,
              status: "idle",
            },
          ];
        },
        corpus_freshness_summary: () => {
          throw new Error("connection refused");
        },
      });
    })(),
  ],
};
