import type { Meta, StoryObj } from "@storybook/react-vite";
import { ProjectManage } from "./ProjectManage";
import type { CorpusInfo } from "../../lib/ipc";
import { withTauriMock } from "../../../.storybook/tauri-mock";

const CORPUS: CorpusInfo = {
  id: "corpus-side",
  display_name: "side-project",
  paths: ["/Users/dev/side-project", "/Users/dev/side-project/docs"],
  status: { state: "idle" },
  files_indexed: 312,
  sections_count: 1290,
  active_sessions: 0,
  model: "bge-small-en-v1.5",
  stack: ["typescript", "rust"],
};

const MODELS = [
  { name: "bge-small-en-v1.5", dimension: 384, description: "fast", code_optimized: false },
  { name: "jina-code-v2", dimension: 768, description: "code", code_optimized: true },
];

const meta = {
  title: "Manager/ProjectManage",
  component: ProjectManage,
} satisfies Meta<typeof ProjectManage>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Healthy index — the manager at rest: status summary + the controls. */
export const Current: Story = {
  args: { corpus: CORPUS, onBack: () => {} },
  decorators: [
    withTauriMock({
      corpus_freshness_summary: { current: 312, stale: 0, new: 0, missing: 0, indexing: false },
      ingestion_progress: [],
      list_supported_models: MODELS,
    }),
  ],
};

/** Behind your changes — the status card shows it; Re-read is right there. */
export const Behind: Story = {
  args: { corpus: CORPUS, onBack: () => {} },
  decorators: [
    withTauriMock({
      corpus_freshness_summary: { current: 308, stale: 2, new: 2, missing: 0, indexing: false },
      ingestion_progress: [],
      list_supported_models: MODELS,
    }),
  ],
};
