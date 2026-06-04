import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { CommandPalette } from "./CommandPalette";
import { withTauriMock } from "../../../.storybook/tauri-mock";
import type { CorpusInfo } from "../../lib/types";

/**
 * CommandPalette — the ⌘K keyboard spine, on the Liquid-Glass tier (DESIGN.md
 * §4). Rendered OPEN over a contentful faux-workspace backdrop so the glass
 * blur + specular highlight are actually visible (glass over a blank canvas
 * shows nothing). `useSessions`/`useEntityPanel` no-op/empty outside their
 * providers; the IPC mock just satisfies the session poll.
 *
 * NOTE: no `component:` on meta — render-based stories would otherwise be
 * forced to supply `args`.
 */

const corpora: CorpusInfo[] = [
  {
    id: "ministr",
    display_name: "ministr",
    paths: ["/Users/alrik/Code/ministr"],
    status: { state: "idle" },
    files_indexed: 1204,
    sections_count: 12840,
    embeddings_count: 12840,
    active_sessions: 1,
    symbols_count: 41902,
    last_indexed: 0,
    model: "jina-code-v2",
  },
  {
    id: "web",
    display_name: "web",
    paths: ["/Users/alrik/Code/web"],
    status: { state: "idle" },
    files_indexed: 320,
    sections_count: 2100,
    embeddings_count: 2100,
    active_sessions: 0,
    symbols_count: 8800,
    last_indexed: 0,
    model: "jina-code-v2",
  },
];

const MOCK = { list_sessions: () => [] };

const meta = {
  title: "Chrome/CommandPalette",
  parameters: { layout: "fullscreen" },
  decorators: [withTauriMock(MOCK)],
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

/** A faux workspace behind the palette so the glass material reads. */
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
              <div className="h-2 w-1/2 rounded bg-border" />
            </div>
          ))}
        </div>
      </div>
      {children}
    </div>
  );
}

const noop = () => {};

export const Open: Story = {
  render: () => (
    <Backdrop>
      <CommandPalette
        open
        onClose={noop}
        corpora={corpora}
        activeCorpusId="ministr"
        onNavigate={noop}
        onSelectCorpus={noop}
        onAddProject={noop}
        onOpenLogs={noop}
        onReindexActive={noop}
        onCycleTheme={noop}
      />
    </Backdrop>
  ),
};
