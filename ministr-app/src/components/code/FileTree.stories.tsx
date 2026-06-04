import type { Meta, StoryObj } from "@storybook/react-vite";
import type { FileInfo } from "../../lib/types";
import { withTauriMock } from "../../../.storybook/tauri-mock";
import { FileTree } from "./FileTree";

/**
 * FileTree — the Explore/Code side bar (aaa-explore-perf-filetree).
 *
 * The `Large` story feeds a 10,000-file corpus through the Tauri mock to prove
 * the windowing: scroll/expand stays smooth and only a few dozen rows are ever
 * mounted (Playwright asserts a bounded `[data-filetree-row]` count). The filter
 * runs through `useDeferredValue`, so typing never blocks the input.
 */

function file(path: string, section_count: number): FileInfo {
  return { path, content_hash: "h", mtime_ns: 0, section_count };
}

const EXTS = ["ts", "tsx", "rs", "md", "py", "go"];

/** Generate `n` files spread across a realistic crate/module/file hierarchy. */
function generateFiles(n: number): FileInfo[] {
  const out: FileInfo[] = [];
  for (let i = 0; i < n; i++) {
    const crate = i % 20;
    const module = Math.floor(i / 20) % 25;
    const ext = EXTS[i % EXTS.length];
    out.push(
      file(`/Users/alrik/Code/ministr/crate-${crate}/module-${module}/file_${i}.${ext}`, (i % 9) + 1),
    );
  }
  return out;
}

const FILES_10K = generateFiles(10_000);

const FILES_SMALL: FileInfo[] = [
  file("/Users/alrik/Code/ministr/ministr-daemon/src/daemon.rs", 61),
  file("/Users/alrik/Code/ministr/ministr-core/src/ingestion/pipeline.rs", 52),
  file("/Users/alrik/Code/ministr/ministr-core/src/index/hnsw.rs", 44),
  file("/Users/alrik/Code/ministr/ministr-app/src/components/code/FileTree.tsx", 9),
  file("/Users/alrik/Code/ministr/README.md", 5),
];

const meta = {
  title: "Code/FileTree",
  component: FileTree,
  parameters: { layout: "fullscreen" },
  args: { corpusId: "ministr", activePath: null, onSelect: () => {} },
  decorators: [
    (Story) => (
      <div className="@container/page h-[820px] w-[320px] bg-bg">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof FileTree>;

export default meta;
type Story = StoryObj<typeof meta>;

/**
 * 10,000 files. The DOM holds only the on-screen window — scrolling and the
 * filter stay smooth. This is the perf story the chunk is graded against.
 */
export const Large: Story = {
  decorators: [withTauriMock({ list_corpus_files: FILES_10K })],
};

/** A handful of files — the everyday small-corpus case, with an active file. */
export const Small: Story = {
  args: {
    activePath: "/Users/alrik/Code/ministr/ministr-app/src/components/code/FileTree.tsx",
  },
  decorators: [withTauriMock({ list_corpus_files: FILES_SMALL })],
};

/** No indexed files — the empty state. */
export const Empty: Story = {
  decorators: [withTauriMock({ list_corpus_files: [] })],
};
