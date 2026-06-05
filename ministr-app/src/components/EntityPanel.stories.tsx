import { useEffect } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { EntityPanel } from "./EntityPanel";
import { EntityPanelProvider, useEntityPanel } from "../hooks/useEntityPanel";
import { withTauriMock } from "../../.storybook/tauri-mock";
import type { CorpusInfo, FileInfo } from "../lib/types";

/**
 * EntityPanel — the universal inspector DRAWER, on the Liquid-Glass drawer tier
 * (DESIGN.md §4, the glassDrawer token). Rendered OPEN (auto-pushed entity) over
 * a contentful faux-workspace backdrop so the glass material is visible through
 * the drawer chrome. Reuses CorpusView's IPC mocks for the body.
 *
 * NOTE: no `component:` on meta — render-based stories would otherwise be
 * forced to supply `args`.
 */

const corpus: CorpusInfo = {
  id: "ministr",
  display_name: "ministr",
  paths: ["/Users/alrik/Code/ministr"],
  status: { state: "idle" },
  files_indexed: 1204,
  sections_count: 12840,
  embeddings_count: 12840,
  active_sessions: 0,
  symbols_count: 41902,
  last_indexed: 0,
  model: "jina-code-v2",
};

const FILES: FileInfo[] = [
  {
    path: "ministr-core/src/retrieval/hybrid.rs",
    content_hash: "a",
    mtime_ns: 0,
    section_count: 31,
  },
];

const MOCK = {
  list_supported_models: () => [
    { name: "jina-code-v2", dimension: 768, code_optimized: true },
  ],
  list_sessions: () => [],
  list_corpus_files: () => FILES,
  recent_coherence_events: () => [],
};

const meta = {
  title: "Chrome/EntityPanel",
  parameters: { layout: "fullscreen" },
  decorators: [withTauriMock(MOCK)],
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

/** Pushes an entity once on mount so the drawer renders open. */
function AutoOpen({ children }: { children: ReactNode }) {
  const { openEntity } = useEntityPanel();
  useEffect(() => {
    openEntity({ kind: "corpus", corpus });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return <>{children}</>;
}

/** A faux workspace behind the drawer so the glass material reads. */
function Backdrop({ children }: { children: ReactNode }) {
  return (
    <div className="relative h-screen w-screen overflow-hidden bg-bg">
      <div className="absolute inset-0 p-6">
        <div className="mb-4 h-12 rounded-lg border border-border bg-surface-raised" />
        <div className="grid grid-cols-3 gap-4">
          {Array.from({ length: 9 }).map((_, i) => (
            <div
              key={i}
              className="space-y-2 rounded-lg border border-border bg-surface p-4"
            >
              <div className="h-3 w-2/3 rounded bg-accent/30" />
              <div className="h-2 w-full rounded bg-border" />
              <div className="h-2 w-5/6 rounded bg-border" />
            </div>
          ))}
        </div>
      </div>
      {children}
    </div>
  );
}

export const Open: Story = {
  render: () => (
    <Backdrop>
      <EntityPanelProvider>
        <AutoOpen>
          <EntityPanel />
        </AutoOpen>
      </EntityPanelProvider>
    </Backdrop>
  ),
};
