import type { Meta, StoryObj } from "@storybook/react-vite";
import type { SolidFinding, SolidSymbolRef } from "../../lib/types";
import { SolidMap } from "./SolidMap";

/**
 * SolidMap — the Explore "Quality" lens (aaa-explore-solid). All six SolidFinding
 * variants normalised into one card, grouped/filterable by principle.
 */

function ref(name: string, kind: string, file: string, line = 42): SolidSymbolRef {
  return { symbol_id: `sym-${file}::${name}`, name, kind, file, line };
}

// NOT exported: in CSF every named export is treated as a story, and a bare
// array isn't a valid story (it would render with no `findings` arg → crash).
const ALL_FINDINGS: SolidFinding[] = [
  {
    type: "redundancy",
    principle: "dry_ocp",
    canonical: ref("handle_get", "function", "ministr-daemon/src/daemon.rs", 410),
    members: [
      ref("handle_get", "function", "ministr-daemon/src/daemon.rs", 410),
      ref("handle_head", "function", "ministr-daemon/src/daemon.rs", 455),
      ref("handle_options", "function", "ministr-daemon/src/daemon.rs", 500),
    ],
    members_total: 3,
    avg_cosine: 0.94,
    avg_jaccard: 0.71,
    cross_module: false,
  },
  {
    type: "low_cohesion",
    principle: "srp",
    container: ref("CorpusRegistry", "struct", "ministr-core/src/registry.rs", 30),
    components: [
      { size: 4, members: [ref("load", "function", "ministr-core/src/registry.rs", 60), ref("persist", "function", "ministr-core/src/registry.rs", 88)] },
      { size: 3, members: [ref("embed", "function", "ministr-core/src/registry.rs", 140), ref("search", "function", "ministr-core/src/registry.rs", 175)] },
    ],
    method_count: 18,
  },
  {
    type: "fat_interface",
    principle: "isp",
    interface: ref("Storage", "trait", "ministr-core/src/storage/traits.rs", 20),
    method_count: 22,
    unused_methods: ["compress", "vacuum", "checkpoint", "snapshot"],
    under_using_implementors: [
      ref("MemoryStorage", "struct", "ministr-core/src/storage/memory.rs", 14),
      ref("ReadOnlyStorage", "struct", "ministr-core/src/storage/ro.rs", 9),
    ],
  },
  {
    type: "concrete_dependency",
    principle: "dip",
    consumer: ref("QueryService", "struct", "ministr-core/src/service/query.rs", 7),
    concrete_target: ref("SqliteStorage", "struct", "ministr-core/src/storage/sqlite.rs", 40),
    suggested_abstraction: ref("Storage", "trait", "ministr-core/src/storage/traits.rs", 20),
  },
  {
    type: "shotgun_surgery",
    principle: "shotgun_surgery",
    name: "to_dto",
    kind: "function",
    sites: [
      ref("to_dto", "function", "ministr-api/src/corpus.rs", 80),
      ref("to_dto", "function", "ministr-api/src/session.rs", 120),
      ref("to_dto", "function", "ministr-api/src/query.rs", 240),
      ref("to_dto", "function", "ministr-app/src-tauri/src/commands.rs", 900),
    ],
    sites_total: 4,
    avg_jaccard: 0.22,
  },
  {
    type: "cyclic_dependency",
    principle: "cyclic_dependency",
    packages: ["ministr-core::service", "ministr-core::index", "ministr-core::storage"],
    edge_count: 6,
    example_edges: [
      { from: "service", to: "index", example_from: ref("survey", "function", "ministr-core/src/service/query.rs", 412), example_to: ref("search", "function", "ministr-core/src/index/hnsw.rs", 88) },
      { from: "index", to: "storage", example_from: ref("rebuild", "function", "ministr-core/src/index/hnsw.rs", 200), example_to: ref("load_vectors", "function", "ministr-core/src/storage/sqlite.rs", 600) },
    ],
  },
];

const noop = () => {};

const meta = {
  title: "Code/SolidMap",
  component: SolidMap,
  parameters: { layout: "fullscreen" },
  args: { onInspect: noop, onOpenFile: noop },
  decorators: [
    (Story) => (
      <div className="h-[820px] w-full bg-bg">
        <div className="mx-auto h-full max-w-4xl border-x border-border bg-surface">
          <Story />
        </div>
      </div>
    ),
  ],
} satisfies Meta<typeof SolidMap>;

export default meta;
type Story = StoryObj<typeof meta>;

/** All six finding variants, normalised into one card each. */
export const Rich: Story = { args: { findings: ALL_FINDINGS } };

/** A tidy architecture — nothing flagged. */
export const Empty: Story = { args: { findings: [] } };

/** First load — auditing. */
export const Loading: Story = { args: { findings: [], loading: true } };
