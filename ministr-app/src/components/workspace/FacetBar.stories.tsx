import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CorpusInfo } from "../../lib/types";
import {
  WorkspaceProvider,
  type FacetId,
  type Spine,
} from "./WorkspaceContext";
import { FacetBar } from "./FacetBar";
import { ScopeHeader } from "./ScopeHeader";

/**
 * The facet switcher as a segmented "deck control". Stories show each facet
 * active (the lifted accent pill + chord hints slide between tabs), plus the
 * combined top chrome (ScopeHeader + FacetBar) to prove they read as one
 * premium frame. Reviewed light + dark.
 */

const HOUR = 3_600;
const nowSec = () => Date.now() / 1000;

const CORPUS: CorpusInfo = {
  id: "ministr",
  display_name: "ministr",
  paths: ["/Users/alrik/Code/ministr"],
  status: { state: "idle" },
  files_indexed: 1284,
  sections_count: 9210,
  embeddings_count: 41233,
  symbols_count: 18422,
  active_sessions: 3,
  last_indexed: nowSec() - 2 * HOUR,
  model: "jina-code-v2",
};

const SPINE: Spine = { kind: "project", id: "ministr" };

function Bar({ facet }: { facet: FacetId }) {
  return (
    <WorkspaceProvider corpora={[CORPUS]} initialSpine={SPINE} initialFacet={facet}>
      <div className="flex h-[220px] w-full flex-col bg-bg">
        <FacetBar />
        <div className="grid flex-1 place-items-center font-mono text-mono-mini text-text-dim">
          {facet} facet body
        </div>
      </div>
    </WorkspaceProvider>
  );
}

const meta = {
  title: "Workspace/FacetBar",
  parameters: { layout: "fullscreen" },
} satisfies Meta;
export default meta;

type Story = StoryObj;

export const Ask: Story = { render: () => <Bar facet="ask" /> };
export const Explore: Story = { render: () => <Bar facet="explore" /> };
export const Activity: Story = { render: () => <Bar facet="activity" /> };
export const Tend: Story = { render: () => <Bar facet="tend" /> };

/** The whole top chrome — the command-deck ScopeHeader over the deck-control
 *  FacetBar — to verify they read as ONE premium frame. */
export const TopChrome: Story = {
  render: () => (
    <WorkspaceProvider
      corpora={[CORPUS]}
      initialSpine={SPINE}
      initialFacet="explore"
    >
      <div className="flex h-[320px] w-full flex-col bg-bg">
        <ScopeHeader />
        <FacetBar />
        <div className="grid flex-1 place-items-center font-mono text-mono-mini text-text-dim">
          facet body
        </div>
      </div>
    </WorkspaceProvider>
  ),
};
