import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CorpusInfo } from "../../lib/types";
import { WorkspaceProvider, type Spine } from "./WorkspaceContext";
import { ScopeHeader } from "./ScopeHeader";

/**
 * The Command Deck — the persistent identity of the workspace's central object
 * (a Project, or Fleet). These stories cover the spine states it renders for:
 * a calm project, a LIVE project (agents attached + glow), mid-index, index
 * error, the Fleet aggregate, and the no-project prompt. Reviewed light + dark.
 */

const HOUR = 3_600;
const nowSec = () => Date.now() / 1000;

function mkCorpus(over: Partial<CorpusInfo> & { id: string }): CorpusInfo {
  return {
    display_name: over.id,
    paths: [`/Users/alrik/Code/${over.id}`],
    status: { state: "idle" },
    files_indexed: 1284,
    sections_count: 9210,
    embeddings_count: 41233,
    symbols_count: 18422,
    active_sessions: 0,
    last_indexed: nowSec() - 2 * HOUR,
    model: "jina-code-v2",
    ...over,
  };
}

const FLEET: CorpusInfo[] = [
  mkCorpus({ id: "ministr", active_sessions: 3 }),
  mkCorpus({ id: "ministr-private", files_indexed: 312, sections_count: 2104, symbols_count: 4210 }),
  mkCorpus({
    id: "atlas-web",
    status: { state: "indexing", files_done: 740, files_total: 1120 },
    files_indexed: 1120,
    symbols_count: 9044,
  }),
  mkCorpus({ id: "rig-engine", files_indexed: 2890, sections_count: 21044, symbols_count: 41200 }),
];

/** Renders the deck over a faux facet body so its lift/shadow + lit top edge
 *  read the way they do in the real shell. */
function Deck({
  corpora,
  spine,
}: {
  corpora: CorpusInfo[];
  spine: Spine;
}) {
  return (
    <WorkspaceProvider corpora={corpora} initialSpine={spine} initialFacet="explore">
      <div className="flex h-[280px] w-full flex-col bg-bg">
        <ScopeHeader />
        <div className="grid flex-1 place-items-center font-mono text-mono-mini text-text-dim">
          facet body
        </div>
      </div>
    </WorkspaceProvider>
  );
}

const meta = {
  title: "Workspace/ScopeHeader",
  parameters: { layout: "fullscreen" },
} satisfies Meta;
export default meta;

type Story = StoryObj;

const P = (id: string): Spine => ({ kind: "project", id });

export const Project: Story = {
  render: () => <Deck corpora={[mkCorpus({ id: "ministr" })]} spine={P("ministr")} />,
};

/** Agents attached → live medallion glow, a LIVE pill, and the live-agents
 *  vital pulses. */
export const ProjectLive: Story = {
  render: () => (
    <Deck
      corpora={[mkCorpus({ id: "ministr", active_sessions: 3 })]}
      spine={P("ministr")}
    />
  ),
};

export const Indexing: Story = {
  render: () => (
    <Deck
      corpora={[
        mkCorpus({
          id: "atlas-web",
          status: { state: "indexing", files_done: 740, files_total: 1120 },
        }),
      ]}
      spine={P("atlas-web")}
    />
  ),
};

export const IndexError: Story = {
  render: () => (
    <Deck
      corpora={[
        mkCorpus({
          id: "ministr",
          status: { state: "error", message: "embedding model failed to load" },
        }),
      ]}
      spine={P("ministr")}
    />
  ),
};

export const Fleet: Story = {
  render: () => <Deck corpora={FLEET} spine={{ kind: "fleet" }} />,
};

export const NoProject: Story = {
  render: () => <Deck corpora={[]} spine={P("missing")} />,
};
