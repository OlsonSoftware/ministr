import type { Meta, StoryObj } from "@storybook/react-vite";
import { within } from "storybook/test";
import { ConnectFlow } from "./ConnectFlow";
import { withTauriMock } from "../../../.storybook/tauri-mock";

const INDEXING = {
  id: "corpus-new",
  display_name: "my-app",
  paths: ["/Users/dev/my-app"],
  status: { state: "indexing", files_done: 412, files_total: 1482 },
  files_indexed: 0,
  sections_count: 0,
  active_sessions: 0,
};

const meta = {
  title: "Screens/ConnectFlow",
  component: ConnectFlow,
} satisfies Meta<typeof ConnectFlow>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Beat 1 — the front door. */
export const Pick: Story = {
  args: { onDone: () => {} },
  decorators: [withTauriMock({})],
};

/** Beat 2 — live plain-words progress (mocked mid-index). */
export const Reading: Story = {
  args: { onDone: () => {} },
  decorators: [
    withTauriMock({
      pick_project_folder: "/Users/dev/my-app",
      register_corpus: { corpus_id: "corpus-new", indexing_started: true },
      list_corpora: [INDEXING],
    }),
  ],
  play: async ({ canvasElement, userEvent }) => {
    const canvas = within(canvasElement);
    const btn = await canvas.findByRole("button", { name: /choose a folder/i });
    await userEvent.click(btn);
    await canvas.findByText(/412 of 1482 files/);
  },
};

/** Beat 3 — waiting for the first REAL tool call (none yet). */
export const ConnectWaiting: Story = {
  args: { onDone: () => {} },
  decorators: [
    withTauriMock({
      pick_project_folder: "/Users/dev/my-app",
      register_corpus: { corpus_id: "corpus-new", indexing_started: true },
      list_corpora: [{ ...INDEXING, status: { state: "idle" }, files_indexed: 1482 }],
      recent_activity: [],
    }),
  ],
  play: async ({ canvasElement, userEvent }) => {
    const canvas = within(canvasElement);
    const btn = await canvas.findByRole("button", { name: /choose a folder/i });
    await userEvent.click(btn);
    await canvas.findByText(/Connect your AI/);
  },
};

/** The handshake — fired by a (mocked) real first tool call. */
export const Handshake: Story = {
  args: { onDone: () => {} },
  decorators: [
    withTauriMock({
      pick_project_folder: "/Users/dev/my-app",
      register_corpus: { corpus_id: "corpus-new", indexing_started: true },
      list_corpora: [{ ...INDEXING, status: { state: "idle" }, files_indexed: 1482 }],
      recent_activity: [
        {
          timestamp_ms: 1_700_000_000_000,
          tool: "ministr_survey",
          corpus_id: "corpus-new",
          summary: "project layout",
          cache_hit: false,
        },
      ],
    }),
  ],
  play: async ({ canvasElement, userEvent }) => {
    const canvas = within(canvasElement);
    const btn = await canvas.findByRole("button", { name: /choose a folder/i });
    await userEvent.click(btn);
    await canvas.findByText(/Your AI just saw your code/);
  },
};
