import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CorpusInfo } from "../../lib/types";
import { FleetDeck } from "./FleetSurface";

/**
 * FleetDeck — the bespoke collection view of the Project object. Vitals at the
 * top; a self-prioritizing constellation of project cells below (live → indexing
 * → freshest → biggest), each with a cross-project relative index-mass bar and
 * an age-toned freshness pip.
 */

const HOUR = 3600;
const DAY = 86_400;
const now = () => Math.floor(Date.now() / 1000);

function mkCorpus(over: Partial<CorpusInfo> & { id: string }): CorpusInfo {
  return {
    display_name: over.id,
    paths: [`/Users/alrik/Code/${over.id}`],
    status: { state: "idle" },
    files_indexed: 800,
    sections_count: 5000,
    embeddings_count: 5000,
    active_sessions: 0,
    symbols_count: 2000,
    last_indexed: now() - 2 * DAY,
    ...over,
  };
}

const FLEET: CorpusInfo[] = [
  mkCorpus({
    id: "ministr",
    files_indexed: 4821,
    sections_count: 38104,
    embeddings_count: 38104,
    symbols_count: 41902,
    active_sessions: 2,
    last_indexed: now() - 2 * HOUR,
  }),
  mkCorpus({
    id: "ministr-app",
    status: { state: "indexing", files_done: 740, files_total: 1284 },
    files_indexed: 1240,
    embeddings_count: 6203,
    symbols_count: 9000,
  }),
  mkCorpus({
    id: "design-system",
    files_indexed: 612,
    embeddings_count: 4188,
    symbols_count: 3100,
    active_sessions: 1,
    last_indexed: now() - 6 * HOUR,
  }),
  mkCorpus({
    id: "legacy-api",
    status: { state: "error", message: "embedding model unavailable" },
    files_indexed: 2204,
    embeddings_count: 14200,
    symbols_count: 18000,
    last_indexed: now() - 9 * DAY,
  }),
  mkCorpus({
    id: "docs-site",
    files_indexed: 188,
    embeddings_count: 980,
    symbols_count: 0,
    last_indexed: now() - 26 * DAY,
  }),
  mkCorpus({
    id: "sandbox",
    warming: true,
    files_indexed: 0,
    sections_count: 0,
    embeddings_count: 0,
    symbols_count: 0,
    last_indexed: undefined,
  }),
];

const noop = () => {};

const meta = {
  title: "Surfaces/FleetSurface",
  component: FleetDeck,
  parameters: { layout: "fullscreen" },
  args: {
    activeCorpusId: null,
    onSelect: noop,
    onAdd: noop,
    onScan: noop,
    onReindex: noop,
    onRemove: noop,
  },
  decorators: [
    (Story) => (
      <div className="h-[760px] w-full bg-bg">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof FleetDeck>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A working fleet: live agents float up, the indexing one shows progress, the
 *  errored one is flagged, mass bars compare index sizes, freshness pips age. */
export const Populated: Story = {
  args: { corpora: FLEET },
};

/** With a project selected as the spine (accent ring + glow). */
export const Selected: Story = {
  args: { corpora: FLEET, activeCorpusId: "design-system" },
};

/** The bespoke STAR-MAP view — projects packed into a constellation, bubble area
 *  ∝ index mass, toned by status, live projects haloed. Click a bubble to zoom. */
export const Constellation: Story = {
  args: { corpora: FLEET, initialView: "map" },
};

/** The star-map with a project selected as the spine (bright accent ring). */
export const ConstellationSelected: Story = {
  args: { corpora: FLEET, initialView: "map", activeCorpusId: "ministr" },
};

/** A cold install — the whole-fleet empty state. */
export const Empty: Story = {
  args: { corpora: [] },
};

/** Several projects indexing at once. */
export const Indexing: Story = {
  args: {
    corpora: [
      mkCorpus({
        id: "ministr",
        status: { state: "indexing", files_done: 980, files_total: 4821 },
        embeddings_count: 38104,
      }),
      mkCorpus({
        id: "ministr-app",
        status: { state: "indexing", files_done: 220, files_total: 1284 },
        embeddings_count: 6203,
      }),
      mkCorpus({ id: "design-system", embeddings_count: 4188, last_indexed: now() - HOUR }),
    ],
  },
};
