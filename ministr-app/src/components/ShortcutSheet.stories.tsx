import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { ShortcutSheet } from "./ShortcutSheet";

/**
 * ShortcutSheet — the ? keyboard-shortcuts help dialog, on the Liquid-Glass
 * tier (DESIGN.md §4, glassPanel). Rendered OPEN over a contentful faux-
 * workspace backdrop so the glass blur + specular are visible. No IPC/provider —
 * the shortcut data is a static lib.
 *
 * NOTE: no `component:` on meta — render-based stories would otherwise be
 * forced to supply `args`.
 */

const meta = {
  title: "Chrome/ShortcutSheet",
  parameters: { layout: "fullscreen" },
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

/** A faux workspace behind the dialog so the glass material reads. */
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

export const Open: Story = {
  render: () => (
    <Backdrop>
      <ShortcutSheet open onClose={noop} />
    </Backdrop>
  ),
};
