import type { ReactNode } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, waitFor, within } from "storybook/test";
import { withTauriMock } from "../../../.storybook/tauri-mock";
import { ToastProvider } from "../shell/ToastTray";
import { EntityPanelProvider } from "../../hooks/useEntityPanel";
import { WorkspaceScreen } from "./WorkspaceScreen";
import { OpenSessionInspector } from "./story-open-inspector";
import { WorkspaceProvider, type FacetId, type Spine } from "./WorkspaceContext";
import {
  LIVE_CORPORA,
  LIVE_FIXTURES,
  LIVE_SESSIONS,
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

function Screen({
  spine,
  facet,
  children,
}: {
  spine: Spine;
  facet: FacetId;
  children?: ReactNode;
}) {
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
          {children}
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

// ── Cross-facet jump e2e: Sessions "code touched" chip → Explore symbol peek ─
//
// Closes the verification gap on aaa-explore-session-codetouched: the jump
// shipped "verified by construction" but the full live click-through was never
// driven end-to-end. This story opens the real session inspector (programmatic
// open — the trigger isn't the gap), then the `play` clicks a real code-touched
// symbol chip and asserts the WHOLE integrated path fired: the inspector closed,
// the Explore facet activated, and the symbol NEIGHBORHOOD PEEK resolved open.
//
// The peek is the discriminator: a search_symbols MISS only re-navigates the
// file (no peek), so asserting the "Neighborhood" peek proves a real resolve
// HIT, not the file-only fallback. Runs in both browser projects → light+dark.

export const CodeTouchedJump: Story = {
  render: () => (
    <Screen spine={{ kind: "project", id: "ministr" }} facet="activity">
      {/* sess-arch-01 — the session the activity feed is scoped to. */}
      <OpenSessionInspector session={LIVE_SESSIONS[0]} />
    </Screen>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);

    // The session inspector renders §1 "Code touched" with a live symbol chip.
    await canvas.findByText("Code touched", undefined, { timeout: 5000 });
    const chip = await canvas.findByRole(
      "button",
      { name: "survey" },
      { timeout: 5000 },
    );

    // Click the chip → cross-facet jump into Explore at that symbol.
    await userEvent.click(chip);

    // The jump landed: the symbol NEIGHBORHOOD peek resolved open AND loaded the
    // symbol's real definition. "Go to definition" only renders when the async
    // symbol_definition resolved non-null — i.e. a real search_symbols HIT
    // (openSymbol), NOT the file-only fallback (which opens no peek at all). This
    // is the discriminating assertion: it can't pass on a mere facet flip.
    await waitFor(
      () =>
        expect(
          canvas.getByRole("button", { name: /go to definition/i }),
        ).toBeInTheDocument(),
      { timeout: 5000 },
    );
    // The peek chrome + the clicked symbol's identity.
    expect(canvas.getByText("Neighborhood")).toBeInTheDocument();
    expect(
      canvas.getByRole("button", { name: "Close neighborhood" }),
    ).toBeInTheDocument();
    expect(canvas.getAllByText("survey").length).toBeGreaterThan(0);

    // We left Sessions: revealInExplore closed the inspector (§1 is gone).
    // waitFor the EntityPanel's AnimatePresence exit to finish unmounting it.
    await waitFor(
      () => expect(canvas.queryByText("Code touched")).not.toBeInTheDocument(),
      { timeout: 5000 },
    );
  },
};
