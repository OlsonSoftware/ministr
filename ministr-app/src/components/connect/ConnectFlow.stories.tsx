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

/** Beat 1 — the front door (returning user adding another project). */
export const Pick: Story = {
  args: { onDone: () => {} },
  decorators: [withTauriMock({})],
};

/** First launch — the gate App.tsx routes a brand-new user into: a
 *  plain-words welcome that says what ministr is, then the same single
 *  "Choose a folder…" action. Never a bare empty Home. */
export const FirstRun: Story = {
  args: { onDone: () => {}, firstRun: true },
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
    // The wait is no longer dead: active verify + troubleshooting + an exit.
    await canvas.findByRole("button", { name: /check connection/i });
    await canvas.findByText(/not working\?/i);
    await canvas.findByRole("button", { name: /skip for now/i });
    // Active verify with no event yet → honest "not connected" verdict.
    await userEvent.click(canvas.getByRole("button", { name: /check connection/i }));
    await canvas.findByText(/hasn’t connected yet/i);
  },
};

/** Beat 3, daemon down — the wait surfaces a recoverable problem, not a
 *  dead screen, when ministr isn't reachable. */
export const ConnectDaemonDown: Story = {
  args: { onDone: () => {} },
  decorators: [
    withTauriMock({
      pick_project_folder: "/Users/dev/my-app",
      register_corpus: { corpus_id: "corpus-new", indexing_started: true },
      list_corpora: [{ ...INDEXING, status: { state: "idle" }, files_indexed: 1482 }],
      recent_activity: () => {
        throw new Error("connection refused");
      },
    }),
  ],
  play: async ({ canvasElement, userEvent }) => {
    const canvas = within(canvasElement);
    const btn = await canvas.findByRole("button", { name: /choose a folder/i });
    await userEvent.click(btn);
    await canvas.findByText(/ministr isn’t running on this Mac/);
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
