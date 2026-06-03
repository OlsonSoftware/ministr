import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CorpusInfo } from "../../lib/types";
import { ProjectsSurface } from "./ProjectsSurface";
import { ToastProvider } from "../shell/ToastTray";
import { surfaceContainer } from "../../lib/ui-tokens";
import { withTauriMock } from "../../../.storybook/tauri-mock";

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

const CORPORA: CorpusInfo[] = [
  corpus({}),
  corpus({
    id: "ministr-private",
    display_name: "ministr-private",
    paths: ["/Users/alrik/Code/ministr-private"],
    files_indexed: 640,
    sections_count: 7100,
    symbols_count: 18400,
    active_sessions: 0,
  }),
  corpus({
    id: "ministr-planning",
    display_name: "ministr-planning",
    paths: ["/Users/alrik/Code/ministr-planning"],
    files_indexed: 42,
    sections_count: 980,
    symbols_count: 120,
    status: { state: "indexing", files_done: 12, files_total: 42 },
  }),
  corpus({
    id: "warming-x",
    display_name: "big-monorepo",
    paths: ["/Users/alrik/Code/big-monorepo"],
    warming: true,
    files_indexed: 0,
    sections_count: 0,
    symbols_count: 0,
  }),
];

function Frame({ children }: { children: React.ReactNode }) {
  return (
    <ToastProvider>
      <div className={surfaceContainer} style={{ height: "100vh" }}>
        <div className="h-full overflow-y-auto p-6">{children}</div>
      </div>
    </ToastProvider>
  );
}

const meta = {
  title: "Surfaces/Projects",
  component: ProjectsSurface,
  parameters: { layout: "fullscreen" },
  decorators: [
    withTauriMock({
      // read commands the Projects view fires — return empty collections
      list_sessions: [],
      linked_projects_list: [],
      recent_activity: [],
    }),
  ],
  args: {
    corpora: CORPORA,
    activeCorpusId: "ministr",
    onSelectCorpus: () => {},
    onRefresh: () => {},
  },
} satisfies Meta<typeof ProjectsSurface>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Populated: Story = {
  render: () => {
    const [active, setActive] = useState<string | null>("ministr");
    return (
      <Frame>
        <ProjectsSurface
          corpora={CORPORA}
          activeCorpusId={active}
          onSelectCorpus={setActive}
          onRefresh={() => {}}
        />
      </Frame>
    );
  },
};

export const Empty: Story = {
  render: () => (
    <Frame>
      <ProjectsSurface
        corpora={[]}
        activeCorpusId={null}
        onSelectCorpus={() => {}}
        onRefresh={() => {}}
      />
    </Frame>
  ),
};
