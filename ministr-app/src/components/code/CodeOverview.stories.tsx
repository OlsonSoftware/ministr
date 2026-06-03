import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CorpusInfo, FileInfo } from "../../lib/types";
import { CodeOverview } from "./CodeOverview";

/**
 * CodeOverview — the Explore facet's entry (aaa-explore-overview). A codebase
 * overview that ties the four lenses together: identity + size, languages, and
 * the CODE INTELLIGENCE deep-link tiles (Bridges / Unused / Quality).
 */

const CORPUS: CorpusInfo = {
  id: "ministr",
  display_name: "ministr",
  paths: ["/Users/alrik/Code/ministr"],
  status: { state: "idle" },
  files_indexed: 1284,
  sections_count: 9210,
  embeddings_count: 41233,
  active_sessions: 3,
  symbols_count: 18422,
  model: "jina-code-v2",
};

function file(path: string, section_count: number): FileInfo {
  return { path, content_hash: "h", mtime_ns: 0, section_count };
}

const FILES: FileInfo[] = [
  file("ministr-daemon/src/daemon.rs", 61),
  file("ministr-core/src/ingestion/pipeline.rs", 52),
  file("ministr-api/src/client.rs", 49),
  file("ministr-core/src/index/hnsw.rs", 44),
  file("ministr-cli/src/commands.rs", 40),
  file("ministr-core/src/service/query.rs", 38),
  file("ministr-mcp/src/server/tools.rs", 33),
  file("ministr-core/src/service/mod.rs", 21),
  file("ministr-app/src/lib/types.ts", 12),
  file("ministr-app/src/components/code/CodeBrowser.tsx", 9),
  file("docs/retrieval.md", 7),
  file("README.md", 5),
];

const noop = () => {};

const meta = {
  title: "Code/CodeOverview",
  component: CodeOverview,
  parameters: { layout: "fullscreen" },
  args: { onOpen: noop, onOpenLens: noop },
  decorators: [
    (Story) => (
      <div className="@container/page h-[820px] w-full bg-bg">
        <div className="mx-auto h-full max-w-4xl border-x border-border bg-surface">
          <Story />
        </div>
      </div>
    ),
  ],
} satisfies Meta<typeof CodeOverview>;

export default meta;
type Story = StoryObj<typeof meta>;

/** The full overview — size, intelligence counts resolved, languages, files. */
export const Rich: Story = {
  args: {
    corpus: CORPUS,
    files: FILES,
    intel: { bridges: 8, unused: 12, quality: 6 },
  },
};

/** Intelligence counts still loading (tiles show ·· but stay clickable). */
export const LoadingIntel: Story = {
  args: {
    corpus: CORPUS,
    files: FILES,
    intel: { bridges: null, unused: null, quality: null },
  },
};

/** A single-language project with a clean graph. */
export const SingleLanguage: Story = {
  args: {
    corpus: { ...CORPUS, display_name: "scripts" },
    files: [file("scripts/build.py", 8), file("scripts/deploy.py", 5)],
    intel: { bridges: 0, unused: 0, quality: 0 },
  },
};
