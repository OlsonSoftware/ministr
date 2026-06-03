import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CorpusInfo, DaemonStatus } from "../../lib/types";
import { withTauriMock } from "../../../.storybook/tauri-mock";
import { ToastProvider } from "../shell/ToastTray";
import { EntityPanelProvider } from "../../hooks/useEntityPanel";
import { WorkspaceScreen } from "./WorkspaceScreen";
import { WorkspaceProvider, type FacetId, type Spine } from "./WorkspaceContext";

/**
 * The LIVE workspace composition — the real shipped surfaces mounted as facets
 * under the shared spine context, exactly as App.tsx renders them. Surfaces run
 * against the Tauri mock (empty/idle states); this proves the facets mount and
 * the spine scopes them, and lets Playwright switch facets + zoom Fleet→project.
 */

function mkCorpus(
  over: Partial<CorpusInfo> & { id: string; paths: string[] },
): CorpusInfo {
  return {
    status: { state: "idle" },
    files_indexed: 0,
    sections_count: 0,
    embeddings_count: 0,
    active_sessions: 0,
    symbols_count: 0,
    ...over,
  };
}

const CORPORA: CorpusInfo[] = [
  mkCorpus({
    id: "ministr",
    display_name: "ministr",
    paths: ["/Users/alrik/Code/ministr"],
    files_indexed: 1284,
    sections_count: 9210,
    embeddings_count: 41233,
    symbols_count: 18422,
    model: "jina-code-v2",
  }),
  mkCorpus({
    id: "ministr-private",
    display_name: "ministr-private",
    paths: ["/Users/alrik/Code/ministr-private"],
    files_indexed: 312,
    sections_count: 2104,
    embeddings_count: 9920,
    symbols_count: 4210,
    model: "jina-code-v2",
  }),
];

const STATUS: DaemonStatus = {
  version: "0.3.1",
  uptime_secs: 8460,
  memory_mb: 412,
  model: "jina-code-v2",
  model_dimension: 768,
  corpora: CORPORA,
  total_sessions: 0,
  log_path: "/Users/alrik/Library/Logs/ministr/daemon.log",
};

const FIXTURES = {
  list_sessions: [],
  list_corpus_files: [],
  read_corpus_activity: [],
  ask_history: [],
};

function Screen({ spine, facet }: { spine: Spine; facet: FacetId }) {
  return (
    <ToastProvider>
      <EntityPanelProvider>
        <WorkspaceProvider
          corpora={CORPORA}
          initialSpine={spine}
          initialFacet={facet}
        >
          <div className="flex flex-col h-full min-h-0">
            <WorkspaceScreen
              status={STATUS}
              error={null}
              theme="dark"
              onThemeChange={() => {}}
              onAddProject={() => {}}
              onOpenLogs={() => {}}
              onShowOnboarding={() => {}}
              onRefresh={() => {}}
            />
          </div>
        </WorkspaceProvider>
      </EntityPanelProvider>
    </ToastProvider>
  );
}

const meta: Meta<typeof WorkspaceScreen> = {
  title: "Workspace/WorkspaceScreen (live)",
  component: WorkspaceScreen,
  parameters: { layout: "fullscreen" },
  decorators: [
    withTauriMock(FIXTURES),
    (Story) => (
      <div className="h-[820px] w-full overflow-hidden rounded-xl border border-border">
        <Story />
      </div>
    ),
  ],
};
export default meta;

type Story = StoryObj<typeof WorkspaceScreen>;

export const Ask: Story = {
  render: () => <Screen spine={{ kind: "project", id: "ministr" }} facet="ask" />,
};
export const Explore: Story = {
  render: () => (
    <Screen spine={{ kind: "project", id: "ministr" }} facet="explore" />
  ),
};
export const Activity: Story = {
  render: () => (
    <Screen spine={{ kind: "project", id: "ministr" }} facet="activity" />
  ),
};
export const Tend: Story = {
  render: () => <Screen spine={{ kind: "project", id: "ministr" }} facet="tend" />,
};
export const Fleet: Story = {
  render: () => <Screen spine={{ kind: "fleet" }} facet="ask" />,
};
