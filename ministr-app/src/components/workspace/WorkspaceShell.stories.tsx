import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CorpusInfo, DaemonStatus } from "../../lib/types";
import { ToastProvider } from "../shell/ToastTray";
import { WorkspaceShell } from "./WorkspaceShell";
import { WorkspaceProvider, type FacetId, type Spine } from "./WorkspaceContext";

/**
 * The integrated workspace shell — the OOUX foundation. These stories are the
 * Playwright-scrutinized proof of the three integration tests:
 *   1. One context — the spine is chosen once; every facet reads it.
 *   2. Switching facets keeps the SAME object in the ScopeHeader.
 *   3. Grows by facet, not destination.
 *
 * Rendered in a fixed frame so the full-height shell reads as an app window.
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
    active_sessions: 2,
    model: "jina-code-v2",
    last_indexed: 1_748_900_000,
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
    last_indexed: 1_748_700_000,
  }),
  mkCorpus({
    id: "ministr-planning",
    display_name: "ministr-planning",
    paths: ["/Users/alrik/Code/ministr-planning"],
    status: { state: "indexing", files_done: 84, files_total: 210 },
    files_indexed: 84,
    sections_count: 640,
    embeddings_count: 2100,
    symbols_count: 0,
    model: "minilm-l6-v2",
  }),
  mkCorpus({
    id: "acme-web",
    display_name: "acme-web",
    paths: ["/Users/alrik/Code/acme-web"],
    warming: true,
  }),
];

const STATUS: DaemonStatus = {
  version: "0.3.1",
  uptime_secs: 8460,
  memory_mb: 412,
  model: "jina-code-v2",
  model_dimension: 768,
  corpora: CORPORA,
  total_sessions: 2,
  log_path: "/Users/alrik/Library/Logs/ministr/daemon.log",
};

function Shell({
  spine,
  facet,
  corpora = CORPORA,
  status = STATUS,
  error = null,
}: {
  spine: Spine;
  facet: FacetId;
  corpora?: CorpusInfo[];
  status?: DaemonStatus | null;
  error?: string | null;
}) {
  return (
    <ToastProvider>
      <WorkspaceProvider corpora={corpora} initialSpine={spine} initialFacet={facet}>
        <WorkspaceShell
          status={status}
          error={error}
          sessionCount={3}
          onOpenLogs={() => {}}
          onOpenPalette={() => {}}
          onAddProject={() => {}}
        />
      </WorkspaceProvider>
    </ToastProvider>
  );
}

const meta: Meta<typeof WorkspaceShell> = {
  title: "Workspace/WorkspaceShell",
  component: WorkspaceShell,
  parameters: { layout: "fullscreen" },
  decorators: [
    (Story) => (
      <div className="h-[820px] w-full overflow-hidden rounded-xl border border-border">
        <Story />
      </div>
    ),
  ],
};
export default meta;

type Story = StoryObj<typeof WorkspaceShell>;

/** A project is the spine; the Ask facet is selected. The default view. */
export const ProjectSelected: Story = {
  render: () => <Shell spine={{ kind: "project", id: "ministr" }} facet="ask" />,
};

/** Zoomed out to the Fleet (collection). Same facet vocabulary, aggregate scope. */
export const FleetSelected: Story = {
  render: () => <Shell spine={{ kind: "fleet" }} facet="activity" />,
};

/** Explore facet — same project object, different verb. */
export const ExploreFacet: Story = {
  render: () => (
    <Shell spine={{ kind: "project", id: "ministr" }} facet="explore" />
  ),
};

/** Tend facet — the care verb on the same object. */
export const TendFacet: Story = {
  render: () => <Shell spine={{ kind: "project", id: "ministr" }} facet="tend" />,
};

/** A project mid-index — the spine reflects live indexing %. */
export const IndexingProject: Story = {
  render: () => (
    <Shell spine={{ kind: "project", id: "ministr-planning" }} facet="ask" />
  ),
};

/** Cold install — no projects yet; the spine offers "Add project". */
export const ColdInstall: Story = {
  render: () => (
    <Shell spine={{ kind: "fleet" }} facet="ask" corpora={[]} status={{ ...STATUS, corpora: [] }} />
  ),
};

/** Daemon unreachable — chrome persists, the daemon dot reads danger. */
export const Disconnected: Story = {
  render: () => (
    <Shell
      spine={{ kind: "project", id: "ministr" }}
      facet="ask"
      status={null}
      error="Can’t reach the ministr daemon"
    />
  ),
};
