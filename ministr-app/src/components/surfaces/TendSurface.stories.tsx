import type { Meta, StoryObj } from "@storybook/react-vite";
import { withTauriMock } from "../../../.storybook/tauri-mock";
import { ToastProvider } from "../shell/ToastTray";
import { WorkspaceProvider, type Spine } from "../workspace/WorkspaceContext";
import type { CorpusInfo } from "../../lib/types";
import { TendSurface } from "./TendSurface";

/**
 * The Tend facet — "look after this project": its freshness/health headline,
 * per-project embedding config, indexed paths, sharing, and a one-keystroke
 * re-index. These stories exercise every freshness verdict the corpusHealth
 * helper produces (FRESH / INDEXED / STALE / NOT INDEXED / INDEXING / INDEX
 * ERROR) so the headline + drift nudge are reviewable (and axe-gated) in both
 * themes, plus the Fleet empty state (nothing selected to tend).
 */

const HOUR = 3_600;
const DAY = 86_400;
const nowSec = () => Date.now() / 1000;

const SUPPORTED_MODELS = [
  { name: "jina-code-v2", dimension: 768, description: "Code-optimised, Matryoshka", code_optimized: true },
  { name: "bge-m3", dimension: 1024, description: "Multilingual dense+sparse", code_optimized: false },
  { name: "all-MiniLM-L6-v2", dimension: 384, description: "Tiny, fast baseline", code_optimized: false },
];

const FIXTURES = {
  list_supported_models: SUPPORTED_MODELS,
  // set_corpus_config / trigger_reindex are user-initiated; default mock → null.
};

function mkCorpus(over: Partial<CorpusInfo> & { id: string }): CorpusInfo {
  return {
    display_name: over.id,
    paths: [`/Users/alrik/Code/${over.id}`],
    status: { state: "idle" },
    files_indexed: 1284,
    sections_count: 9210,
    embeddings_count: 41233,
    symbols_count: 18422,
    active_sessions: 2,
    last_indexed: nowSec() - 2 * HOUR,
    model: "jina-code-v2",
    ...over,
  };
}

function Surface({
  corpus,
  fleet = false,
}: {
  corpus: CorpusInfo;
  fleet?: boolean;
}) {
  const spine: Spine = fleet
    ? { kind: "fleet" }
    : { kind: "project", id: corpus.id };
  return (
    <ToastProvider>
      <WorkspaceProvider
        corpora={[corpus]}
        initialSpine={spine}
        initialFacet="tend"
      >
        <div className="h-[760px] w-full overflow-hidden rounded-xl border border-border">
          <TendSurface onRefresh={() => {}} />
        </div>
      </WorkspaceProvider>
    </ToastProvider>
  );
}

const meta = {
  title: "Surfaces/TendSurface",
  parameters: { layout: "fullscreen" },
  decorators: [withTauriMock(FIXTURES)],
} satisfies Meta;
export default meta;

type Story = StoryObj;

/** < 1 day → FRESH (success). Index is up to date. */
export const Fresh: Story = {
  render: () => (
    <Surface corpus={mkCorpus({ id: "ministr", last_indexed: nowSec() - 2 * HOUR })} />
  ),
};

/** 1–7 days → INDEXED (accent). Recent, still current. */
export const Indexed: Story = {
  render: () => (
    <Surface corpus={mkCorpus({ id: "ministr", last_indexed: nowSec() - 3 * DAY })} />
  ),
};

/** > 7 days → STALE (warning) — the headline nudges a re-index. */
export const Stale: Story = {
  render: () => (
    <Surface corpus={mkCorpus({ id: "ministr", last_indexed: nowSec() - 30 * DAY })} />
  ),
};

/** Never indexed → NOT INDEXED (muted) — re-index = first index. */
export const NotIndexed: Story = {
  render: () => (
    <Surface
      corpus={mkCorpus({
        id: "fresh-clone",
        last_indexed: undefined,
        files_indexed: 0,
        sections_count: 0,
        embeddings_count: 0,
        symbols_count: 0,
        active_sessions: 0,
      })}
    />
  ),
};

/** Indexing → INDEXING (accent) — headline + the live progress tray. */
export const Indexing: Story = {
  render: () => (
    <Surface
      corpus={mkCorpus({
        id: "ministr",
        status: { state: "indexing", files_done: 740, files_total: 1120 },
      })}
    />
  ),
};

/** Index error → INDEX ERROR (danger) — headline nudges a retry. */
export const IndexError: Story = {
  render: () => (
    <Surface
      corpus={mkCorpus({
        id: "ministr",
        status: { state: "error", message: "embedding model failed to load" },
      })}
    />
  ),
};

/** Fleet spine → nothing selected to tend → the redirect empty state. */
export const FleetEmpty: Story = {
  render: () => <Surface corpus={mkCorpus({ id: "ministr" })} fleet />,
};
