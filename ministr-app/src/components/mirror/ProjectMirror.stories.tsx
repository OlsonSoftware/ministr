import type { Meta, StoryObj } from "@storybook/react-vite";
import { ProjectMirror } from "./ProjectMirror";
import { withTauriMock } from "../../../.storybook/tauri-mock";

const CORPUS = {
  id: "corpus-bbbb",
  display_name: "side-project",
  paths: ["/Users/dev/side-project"],
  status: "idle",
  files_indexed: 6,
  sections_count: 412,
  active_sessions: 2,
};

const FRESHNESS = {
  files: [
    { path: "src/components/LoginForm.tsx", state: "current" },
    { path: "src/components/Navbar.tsx", state: "current" },
    { path: "src/lib/auth.ts", state: "stale" },
    { path: "src/lib/db.ts", state: "current" },
    { path: "src/pages/index.tsx", state: "current" },
    { path: "README.md", state: "new" },
  ],
  indexing: false,
};

const meta = {
  title: "Screens/ProjectMirror",
  component: ProjectMirror,
  decorators: [
    withTauriMock({
      corpus_freshness: FRESHNESS,
    }),
  ],
} satisfies Meta<typeof ProjectMirror>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Behind: Story = {
  args: { corpus: CORPUS, onBack: () => {} },
};

export const AllCurrent: Story = {
  args: {
    corpus: { ...CORPUS, display_name: "my-app", active_sessions: 0 },
    onBack: () => {},
  },
  decorators: [
    withTauriMock({
      corpus_freshness: {
        files: FRESHNESS.files.map((f) => ({ ...f, state: "current" })),
        indexing: false,
      },
    }),
  ],
  render: (args) => <ProjectMirror {...args} onBack={() => {}} />,
};

export const LivePresence: Story = {
  args: { corpus: CORPUS, onBack: () => {} },
  decorators: [
    withTauriMock({
      corpus_freshness: FRESHNESS,
      recent_activity: () => [
        {
          timestamp_ms: Date.now() - 2_000,
          tool: "ministr_read",
          corpus_id: "corpus-bbbb",
          summary: "src/components/LoginForm.tsx",
          cache_hit: false,
        },
      ],
    }),
  ],
};

export const Updating: Story = {
  args: { corpus: CORPUS, onBack: () => {} },
  decorators: [
    withTauriMock({
      corpus_freshness: { ...FRESHNESS, indexing: true },
    }),
  ],
};
