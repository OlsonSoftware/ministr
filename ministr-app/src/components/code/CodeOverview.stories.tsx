import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CorpusInfo, FileInfo } from "../../lib/types";
import { CodeOverview } from "./CodeOverview";

/**
 * CodeOverview — the Explore facet's entry, a command-deck CODEBASE OBSERVATORY
 * (aaa-codeoverview-observatory). A glowing identity hero (medallion + name +
 * LIVE) over a divided vital readout, the CODE INTELLIGENCE deck (Bridges /
 * Unused / Quality), an accent-ramp language viz, and quick-start files.
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

/** The full overview — live corpus (3 sessions → glowing medallion + LIVE
 *  pill), size readout, resolved intelligence counts, languages, files. */
export const Rich: Story = {
  args: {
    corpus: CORPUS,
    files: FILES,
    intel: { bridges: 8, unused: 12, quality: 6 },
  },
};

/** A quiet corpus with no active sessions — the medallion + dot go neutral
 *  (no glow), the LIVE pill is absent. */
export const Quiet: Story = {
  args: {
    corpus: { ...CORPUS, active_sessions: 0 },
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

/** A freshly-registered corpus with nothing indexed yet — zero size, no
 *  languages section, the empty quick-start message. */
export const Empty: Story = {
  args: {
    corpus: {
      ...CORPUS,
      display_name: "new-project",
      files_indexed: 0,
      sections_count: 0,
      symbols_count: 0,
      active_sessions: 0,
    },
    files: [],
    intel: { bridges: 0, unused: 0, quality: 0 },
  },
};
