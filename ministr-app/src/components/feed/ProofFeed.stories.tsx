import type { Meta, StoryObj } from "@storybook/react-vite";
import { ProofFeed } from "./ProofFeed";
import { withTauriMock } from "../../../.storybook/tauri-mock";

const CORPUS = {
  id: "corpus-bbbb",
  display_name: "side-project",
  paths: ["/Users/dev/side-project"],
  status: "idle",
  files_indexed: 6,
  model: "minilm",
  sections_count: 412,
  active_sessions: 1,
};

const ACTIVITY = [
  {
    timestamp_ms: 1_700_000_300_000,
    tool: "ministr_definition",
    corpus_id: "corpus-bbbb",
    session_id: "s1",
    summary: "auth::handleSubmit",
    cache_hit: false,
  },
  {
    timestamp_ms: 1_700_000_200_000,
    tool: "ministr_read",
    corpus_id: "corpus-bbbb",
    session_id: "s1",
    summary: "src/components/LoginForm.tsx",
    tokens_delta: 2100,
    cache_hit: false,
  },
  {
    timestamp_ms: 1_700_000_100_000,
    tool: "ministr_survey",
    corpus_id: "corpus-bbbb",
    session_id: "s1",
    summary: "login button",
    cache_hit: true,
  },
];

const OUTCOMES = {
  events: [
    {
      session_id: "s1",
      path: "/r/src/components/LoginForm.tsx",
      read_rank: 1,
      first_touch: true,
      reads_before: 0,
      edited_at_ms: 1_700_000_400_000,
    },
    {
      session_id: "s1",
      path: "/r/src/lib/auth.ts",
      read_rank: 3,
      first_touch: false,
      reads_before: 2,
      edited_at_ms: 1_700_000_250_000,
    },
  ],
  stats: [
    { session_id: "s1", distinct_reads: 3, joins: 2, first_touch_hits: 1 },
  ],
};

const meta = {
  title: "Screens/ProofFeed",
  component: ProofFeed,
  decorators: [
    withTauriMock({
      recent_activity: ACTIVITY,
      corpus_outcomes: OUTCOMES,
    }),
  ],
} satisfies Meta<typeof ProofFeed>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Working: Story = {
  args: { corpus: CORPUS, onBack: () => {}, backLabel: "All projects" },
};

export const Empty: Story = {
  args: { corpus: CORPUS, onBack: () => {}, backLabel: "All projects" },
  decorators: [
    withTauriMock({
      recent_activity: [],
      corpus_outcomes: { events: [], stats: [] },
    }),
  ],
};
