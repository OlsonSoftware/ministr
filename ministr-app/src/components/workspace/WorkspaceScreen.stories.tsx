import type { Meta, StoryObj } from "@storybook/react-vite";
import { withTauriMock } from "../../../.storybook/tauri-mock";
import { ToastProvider } from "../shell/ToastTray";
import { EntityPanelProvider } from "../../hooks/useEntityPanel";
import { WorkspaceScreen } from "./WorkspaceScreen";
import { WorkspaceProvider, type FacetId, type Spine } from "./WorkspaceContext";
import {
  LIVE_CORPORA,
  LIVE_FIXTURES,
  LIVE_STATUS,
  seedAskThreads,
} from "./live-fixtures";

/**
 * The LIVE workspace composition — the real shipped surfaces mounted as facets
 * under the shared spine context, exactly as App.tsx renders them, but driven
 * by a RICH Tauri-mock fixture bundle (`live-fixtures`) so every facet renders
 * POPULATED rather than empty:
 *
 *   • Fleet     — a five-project constellation (ready / indexing / warming)
 *   • Activity  — a live session board incl. a nested subagent lineage
 *   • Explore   — a populated file tree + landing; click a file → a symbol to
 *                 open the SymbolNeighborhood peek (driven in Playwright)
 *   • Tend      — the spine project's health + the embedding-model picker
 *   • Ask       — seeded conversation History + Pinned answers (localStorage)
 *
 * This proves the whole workspace end-to-end: the facets mount, the spine
 * scopes them, and Playwright can switch facets + zoom Fleet→project.
 */

function Screen({ spine, facet }: { spine: Spine; facet: FacetId }) {
  return (
    <ToastProvider>
      <EntityPanelProvider>
        <WorkspaceProvider
          corpora={LIVE_CORPORA}
          initialSpine={spine}
          initialFacet={facet}
        >
          <div className="flex flex-col h-full min-h-0">
            <WorkspaceScreen
              status={LIVE_STATUS}
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
    withTauriMock(LIVE_FIXTURES),
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
  render: () => {
    // Seed the per-corpus thread store before the surface mounts so its
    // History rail + Pinned section load populated (they read localStorage,
    // not a Tauri command).
    seedAskThreads();
    return <Screen spine={{ kind: "project", id: "ministr" }} facet="ask" />;
  },
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
