import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CorpusInfo, DaemonStatus } from "../../../lib/types";
import { AskSurface } from "./AskSurface";
import { surfaceContainer } from "../../../lib/ui-tokens";
import { withTauriMock } from "../../../../.storybook/tauri-mock";

/**
 * AskSurface — the flagship conversational Q&A surface.
 *
 * The streaming/done states are driven by a Tauri `Channel` the IPC mock
 * can't pump, so this file covers the surface's *reachable-on-mount* states
 * (ready, inference-down, no-project). The per-answer / per-phase states
 * (AskAnswer, AskStatus, …) have their own stories in `AskStates.stories.tsx`.
 */

const corpus = (over: Partial<CorpusInfo>): CorpusInfo => ({
  id: "ministr",
  display_name: "ministr",
  paths: ["/Users/alrik/Code/ministr"],
  status: { state: "idle" },
  files_indexed: 1204,
  sections_count: 12840,
  embeddings_count: 12840,
  active_sessions: 1,
  symbols_count: 41902,
  last_indexed: Date.now() - 3_600_000,
  model: "jina-code-v2",
  ...over,
});

const status = (corpora: CorpusInfo[]): DaemonStatus => ({
  version: "0.2.1",
  uptime_secs: 4210,
  memory_mb: 612,
  model: "jina-code-v2",
  model_dimension: 768,
  corpora,
  total_sessions: 3,
});

const HEALTHY = {
  inference_health: { available: true, reason: "", binary_path: "/opt/claude" },
};
const NO_CLI = {
  inference_health: {
    available: false,
    reason: "The `claude` binary was not found on your PATH.",
    binary_path: null,
  },
};

function Frame({ children }: { children: React.ReactNode }) {
  return (
    <div className={surfaceContainer} style={{ height: "100vh" }}>
      {children}
    </div>
  );
}

const meta = {
  title: "Surfaces/Ask",
  component: AskSurface,
  parameters: { layout: "fullscreen" },
  decorators: [
    (Story) => (
      <Frame>
        <Story />
      </Frame>
    ),
  ],
} satisfies Meta<typeof AskSurface>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A project is selected and inference is healthy — starter chips + the
 *  1180px+ pinned-answers sidebar. */
export const Ready: Story = {
  decorators: [withTauriMock(HEALTHY)],
  args: {
    status: status([corpus({})]),
    activeCorpusId: "ministr",
  },
};

/** The Claude CLI is missing — the input is disabled and the body explains
 *  what's needed. */
export const InferenceUnavailable: Story = {
  decorators: [withTauriMock(NO_CLI)],
  args: {
    status: status([corpus({})]),
    activeCorpusId: "ministr",
  },
};

/** No project indexed yet — the surface owns its own empty handling and
 *  routes the user toward the Add flow. */
export const NoProject: Story = {
  decorators: [withTauriMock(HEALTHY)],
  args: {
    status: status([]),
    activeCorpusId: null,
  },
};
