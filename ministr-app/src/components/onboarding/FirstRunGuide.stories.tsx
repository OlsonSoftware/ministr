import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CorpusInfo } from "../../lib/types";
import { FirstRunGuide } from "./FirstRunGuide";

/**
 * FirstRunGuide — first value INSIDE the workspace (aaa-onboarding). The three
 * states follow the live corpus list: welcome (no projects) → indexing (live
 * determinate progress) → ask (ready). Rendered over a mock workspace bg so the
 * "inside the workspace" framing reads.
 */

function mkCorpus(over: Partial<CorpusInfo> & { id: string }): CorpusInfo {
  return {
    paths: [`/Users/alrik/Code/${over.id}`],
    display_name: over.id,
    status: { state: "idle" },
    files_indexed: 0,
    sections_count: 0,
    embeddings_count: 0,
    active_sessions: 0,
    symbols_count: 0,
    ...over,
  };
}

const noop = () => {};

const meta = {
  title: "Onboarding/FirstRunGuide",
  component: FirstRunGuide,
  parameters: { layout: "fullscreen" },
  args: {
    onPickFolder: noop,
    onAutoDetect: noop,
    onAsk: noop,
    onSkip: noop,
  },
  decorators: [
    (Story) => (
      // A faint mock of the workspace behind the scrim.
      <div className="relative h-[760px] w-full bg-bg">
        <div className="h-12 border-b border-border bg-surface" />
        <div className="grid h-[calc(100%-3rem)] place-items-center font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
          workspace
        </div>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof FirstRunGuide>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Step 1 — value-first: point at a folder. */
export const Welcome: Story = {
  args: { corpora: [] },
};

/** Step 1 with a detect/pick in flight. */
export const WelcomeBusy: Story = {
  args: { corpora: [], busy: true },
};

/** Step 2 — live indexing with real determinate progress. */
export const Indexing: Story = {
  args: {
    corpora: [
      mkCorpus({
        id: "ministr",
        status: { state: "indexing", files_done: 740, files_total: 1284 },
      }),
    ],
  },
};

/** Step 3 — the project is ready; the first ask is part of onboarding. */
export const Ask: Story = {
  args: {
    corpora: [
      mkCorpus({
        id: "ministr",
        status: { state: "idle" },
        files_indexed: 1284,
        sections_count: 9210,
        embeddings_count: 41233,
        symbols_count: 18422,
      }),
    ],
  },
};
