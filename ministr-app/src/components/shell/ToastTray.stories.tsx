import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { ToastItem, ToastProvider, useToast } from "./ToastTray";

/**
 * ToastTray — transient notifications on the Liquid-Glass tier (DESIGN.md §4,
 * glassPanel). Each toast carries a per-severity command-deck identity: a quiet
 * tone medallion, a tone left-spine, a title/detail hierarchy, and a countdown
 * bar that paces the auto-dismiss (and pauses on hover/focus).
 *
 * Rendered bottom-left over a contentful faux-workspace backdrop so the glass
 * blur + specular read. `Severities` is static (no-op dismiss → persistent) for
 * the visual + a11y gate; `Interactive` exercises the real lifecycle.
 *
 * NOTE: no `component:` on meta — render-based stories would otherwise be forced
 * to supply `args`.
 */

const meta = {
  title: "Chrome/ToastTray",
  parameters: { layout: "fullscreen" },
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

/** A faux workspace behind the tray so the glass material reads. */
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

const noop = () => {};

/** All three severities, stacked in the real bottom-left tray position. The
 *  no-op dismiss keeps them on screen (the countdown still drains) so the glass
 *  + tone language is reviewable and axe can run. */
export const Severities: Story = {
  render: () => (
    <Backdrop>
      <div className="fixed bottom-4 left-4 z-[1100] flex flex-col gap-2">
        <ToastItem
          toast={{ id: 1, tone: "info", label: "Re-indexing ministr-core…" }}
          onDismiss={noop}
        />
        <ToastItem
          toast={{
            id: 2,
            tone: "success",
            label: "Project added",
            detail: "~/Code/ministr · 1,284 symbols indexed",
          }}
          onDismiss={noop}
        />
        <ToastItem
          toast={{
            id: 3,
            tone: "danger",
            label: "Could not open log file",
            detail: "EACCES: permission denied",
          }}
          onDismiss={noop}
        />
      </div>
    </Backdrop>
  ),
};

function Triggers() {
  const { toast } = useToast();
  const btn =
    "rounded-md border border-border bg-surface px-3 py-1.5 font-mono text-xs text-text-muted hover:text-text hover:bg-surface-overlay transition-colors duration-150";
  return (
    <div className="fixed left-1/2 top-1/2 z-[1200] flex -translate-x-1/2 -translate-y-1/2 gap-2">
      <button className={btn} onClick={() => toast("Re-indexing ministr-core…")}>
        info
      </button>
      <button
        className={btn}
        onClick={() =>
          toast("Project added", {
            tone: "success",
            detail: "~/Code/ministr · 1,284 symbols indexed",
          })
        }
      >
        success
      </button>
      <button
        className={btn}
        onClick={() =>
          toast("Could not open log file", {
            tone: "danger",
            detail: "EACCES: permission denied",
          })
        }
      >
        danger
      </button>
    </div>
  );
}

/** The real provider + lifecycle: click a trigger to fire a toast; hover a
 *  toast to pause its countdown. */
export const Interactive: Story = {
  render: () => (
    <ToastProvider>
      <Backdrop>
        <Triggers />
      </Backdrop>
    </ToastProvider>
  ),
};
