import type { Meta, StoryObj } from "@storybook/react-vite";
import { IndexingInstrument } from "./IndexingInstrument";
import { IndexingInstrumentLive } from "./IndexingInstrumentLive";
import type { DerivedProgress } from "../../lib/progress";
import { withTauriMock } from "../../../.storybook/tauri-mock";
import { fullRunScript } from "../../lib/progressMockScripts";

/**
 * Static fixtures drive every phase/edge deterministically (mechanical
 * fill-width probes need fixed values, not a live script); the one Live
 * story replays the scripted full run through the real hook.
 */
function derived(over: Partial<DerivedProgress>): DerivedProgress {
  return {
    corpusId: "corpus-demo",
    phase: "embedding",
    running: true,
    complete: false,
    filesDone: 240,
    filesTotal: 240,
    embeddingsDone: 2604,
    embeddingsTotal: 4200,
    currentFile: "src/daemon/indexer.rs",
    percent: 0.62,
    ratePerSec: 210,
    etaSeconds: 8,
    stalled: false,
    ...over,
  };
}

const meta = {
  title: "UI/IndexingInstrument",
  component: IndexingInstrument,
} satisfies Meta<typeof IndexingInstrument>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Mid-embed: two segments full, the third 62% and breathing, ETA live. */
export const Embedding: Story = {
  args: { progress: derived({}) },
};

/** Early parse: first segment full, second filling, embeddings untouched. */
export const Parsing: Story = {
  args: {
    progress: derived({
      phase: "parsing",
      filesDone: 96,
      filesTotal: 240,
      embeddingsDone: 0,
      percent: 0.4,
      ratePerSec: 48,
      etaSeconds: 3,
      currentFile: "src/lib/quorum.rs",
    }),
  },
};

/** Discovery with no totals yet: honest indeterminate sweep, no fake %. */
export const Discovering: Story = {
  args: {
    progress: derived({
      phase: "discovering",
      filesDone: 0,
      filesTotal: 0,
      embeddingsDone: 0,
      embeddingsTotal: 0,
      percent: null,
      ratePerSec: null,
      etaSeconds: null,
      currentFile: null,
    }),
  },
};

/** Stalled mid-run: the ETA hides; "still working…" replaces it. */
export const Stalled: Story = {
  args: {
    progress: derived({
      stalled: true,
      etaSeconds: null,
      ratePerSec: null,
    }),
  },
};

/** Complete: the track resolves into the trust-green state. */
export const Complete: Story = {
  args: {
    progress: derived({
      running: false,
      complete: true,
      phase: "idle",
      embeddingsDone: 4200,
      percent: 1,
      ratePerSec: null,
      etaSeconds: null,
      currentFile: null,
    }),
  },
};

/** Compact inline variant — the track alone, as TrustPanel rows mount it. */
export const Compact: Story = {
  args: { progress: derived({}), variant: "compact" },
};

/** The full scripted run replayed through the real poll → derive path. */
export const Live: Story = {
  args: { progress: derived({}) },
  decorators: [withTauriMock({ ingestion_progress: fullRunScript() })],
  render: () => <IndexingInstrumentLive />,
};
