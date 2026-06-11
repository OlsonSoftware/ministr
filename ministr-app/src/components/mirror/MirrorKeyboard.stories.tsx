import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, waitFor, within } from "storybook/test";
import { ProjectMirror } from "./ProjectMirror";
import { withTauriMock } from "../../../.storybook/tauri-mock";

const CORPUS = {
  id: "corpus-bbbb",
  display_name: "my-app",
  paths: ["/u/me/my-app"],
  files_indexed: 3,
  model: "minilm",
  sections_count: 9,
  active_sessions: 0,
  status: "idle",
};

const FRESHNESS = {
  files: [
    { path: "src/login.tsx", state: "stale" },
    { path: "src/ok.ts", state: "current" },
    { path: "docs/readme.md", state: "current" },
  ],
  indexing: false,
};

const meta = {
  title: "Screens/ProjectMirror/Keyboard",
  component: ProjectMirror,
  decorators: [
    withTauriMock({
      corpus_freshness: FRESHNESS,
      recent_activity: [],
      indexed_file: { found: true, sections: [{ heading: "", text: "x" }] },
    }),
  ],
} satisfies Meta<typeof ProjectMirror>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Arrows rove the tree; Enter opens a drill-in; Escape closes it and
 *  restores focus to the row that opened it. */
export const ArrowsEnterEscape: Story = {
  args: { corpus: CORPUS, onBack: () => {}, onOpenFeed: () => {} },
  play: async ({ canvasElement, userEvent }) => {
    const canvas = within(canvasElement);
    // wait for the tree, focus the first row
    const first = await canvas.findByRole("button", { name: /src\// });
    first.focus();
    // ArrowDown moves focus to the next row
    await userEvent.keyboard("{ArrowDown}");
    const active = document.activeElement as HTMLElement;
    await expect(active.hasAttribute("data-tree-row")).toBe(true);
    await expect(active).not.toBe(first);
    // walk to a FILE row, note its path, Enter opens the drill-in
    let el = document.activeElement as HTMLElement;
    while (!el.hasAttribute("data-tree-path")) {
      await userEvent.keyboard("{ArrowDown}");
      el = document.activeElement as HTMLElement;
    }
    const path = el.getAttribute("data-tree-path");
    await userEvent.keyboard("{Enter}");
    await canvas.findByLabelText(/back to the file tree/);
    // Escape closes it and restores focus to the same row
    await userEvent.keyboard("{Escape}");
    await waitFor(() => {
      const restored = document.activeElement as HTMLElement;
      if (restored.getAttribute("data-tree-path") !== path) {
        throw new Error("focus not restored to the opening row");
      }
    });
  },
};
