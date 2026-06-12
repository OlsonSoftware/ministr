import type { Meta, StoryObj } from "@storybook/react-vite";
import { ProgressProbe } from "./ProgressProbe";
import { withTauriMock } from "../../../.storybook/tauri-mock";
import {
  completeScript,
  fullRunScript,
  stallScript,
  steadyEmbedScript,
} from "../../lib/progressMockScripts";

/**
 * Live exercise of useIngestionProgress against scripted daemon snapshots
 * (gui-progress-data-hook). Each story replays a progress scenario through
 * the real poll → tracker → derive path; the probe renders every derived
 * field so the honesty rules are visible in motion (ETA appears only after
 * the rate stabilizes, hides on stall, percent never lies).
 */
const meta = {
  title: "Lib/IngestionProgress",
  component: ProgressProbe,
  args: { intervalMs: 300 },
} satisfies Meta<typeof ProgressProbe>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Discover → parse → embed → complete, end to end. */
export const FullRun: Story = {
  decorators: [withTauriMock({ ingestion_progress: fullRunScript() })],
};

/** Mid-run embedding at a steady rate — percent, rate and ETA all live. */
export const SteadyEmbedding: Story = {
  decorators: [withTauriMock({ ingestion_progress: steadyEmbedScript() })],
};

/** Progress freezes mid-run: after ~5s the ETA hides and "stalled" shows. */
export const Stall: Story = {
  decorators: [withTauriMock({ ingestion_progress: stallScript() })],
};

/** A finished run beside an idle corpus — terminal states. */
export const CompleteAndIdle: Story = {
  decorators: [withTauriMock({ ingestion_progress: completeScript() })],
};

/** No corpora reporting at all. */
export const Empty: Story = {
  decorators: [withTauriMock({ ingestion_progress: [] })],
};
