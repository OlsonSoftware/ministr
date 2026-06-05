import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { userEvent, within } from "storybook/test";
import type { DaemonStatus } from "../../lib/types";
import { LIVE_CORPORA, LIVE_STATUS } from "../workspace/live-fixtures";
import { DaemonDot } from "./DaemonDot";

/**
 * DaemonDot — the TopBar daemon status control + its vitals popover, on the
 * Liquid-Glass tier (DESIGN.md §4, glassPanel). Each story auto-opens the
 * popover (play → click) over a contentful faux-workspace backdrop so the glass
 * blur/specular read AND axe runs on the open state. Reuses the live fixtures
 * for realistic vitals.
 *
 * NOTE: no `component:` on meta — render-based stories would otherwise be forced
 * to supply `args`.
 */

const meta = {
  title: "Chrome/DaemonDot",
  parameters: { layout: "fullscreen" },
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

const LOG = "/Users/you/.ministr/ministrd.log";

const ready: DaemonStatus = { ...LIVE_STATUS, corpora: [], log_path: LOG };

const indexing: DaemonStatus = {
  ...LIVE_STATUS,
  log_path: LOG,
  // .map (not index access) keeps this type-safe regardless of fixture length.
  corpora: LIVE_CORPORA.slice(0, 1).map((c) => ({
    ...c,
    status: { state: "indexing" as const, files_done: 4, files_total: 12 },
  })),
};

/** Faux top bar so the popover anchors under a right-aligned trigger, the way
 *  the real TopBar mounts it. */
function Backdrop({ children }: { children: ReactNode }) {
  return (
    <div className="relative h-screen w-screen overflow-hidden bg-bg">
      {/* Centered so the right-anchored popover is unambiguously in-frame
          (the story showcases the popover on glass, not exact TopBar placement). */}
      <div className="flex h-12 items-center justify-center gap-2 border-b border-border bg-surface-raised px-4">
        {children}
      </div>
      <div className="grid grid-cols-3 gap-4 p-6">
        {Array.from({ length: 6 }).map((_, i) => (
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
  );
}

const openPopover = async ({
  canvasElement,
}: {
  canvasElement: HTMLElement;
}) => {
  const canvas = within(canvasElement);
  await userEvent.click(await canvas.findByRole("button", { name: /Daemon/ }));
};

const noop = () => {};

/** Connected + idle → success tone, full vitals + log action. */
export const Ready: Story = {
  render: () => (
    <Backdrop>
      <DaemonDot status={ready} error={null} onOpenLogs={noop} />
    </Backdrop>
  ),
  play: openPopover,
};

/** A corpus indexing → warning tone (pulsing trigger dot, warning pill). */
export const Indexing: Story = {
  render: () => (
    <Backdrop>
      <DaemonDot status={indexing} error={null} onOpenLogs={noop} />
    </Backdrop>
  ),
  play: openPopover,
};

/** No daemon + an error → danger tone, error inset, no log action. */
export const Offline: Story = {
  render: () => (
    <Backdrop>
      <DaemonDot
        status={null}
        error="Daemon socket not reachable: ECONNREFUSED ~/.ministr/ministrd.sock"
      />
    </Backdrop>
  ),
  play: openPopover,
};
