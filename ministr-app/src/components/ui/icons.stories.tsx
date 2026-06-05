import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ComponentType, SVGProps } from "react";
import * as Icons from "./icons";

/**
 * The ministr icon family — every glyph the app draws from, realized over
 * Iconoir's distinctive 1.5px hairline (see `./icons.tsx`). This gallery is the
 * living catalog + a visual-regression surface: if a glyph re-points, it shows
 * here first.
 */
type IconCmp = ComponentType<SVGProps<SVGSVGElement> & { strokeWidth?: number }>;

const ALL = (Object.entries(Icons) as [string, IconCmp][])
  .filter(([, v]) => typeof v === "function" || typeof v === "object")
  .sort((a, b) => a[0].localeCompare(b[0]));

const meta = {
  title: "UI/Icons",
  parameters: { layout: "fullscreen" },
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

/** The whole family, named — the app's icon vocabulary at a glance. */
export const Family: Story = {
  render: () => (
    <div className="min-h-screen bg-surface-sunken p-6">
      <div className="mb-4">
        <h2 className="text-lg font-semibold tracking-tight text-text">
          ministr icons
        </h2>
        <p className="text-sm text-text-dim">
          {ALL.length} glyphs · Iconoir · one swap point in{" "}
          <span className="font-mono text-text-muted">ui/icons.tsx</span>
        </p>
      </div>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(7rem,1fr))] gap-2">
        {ALL.map(([name, Icon]) => (
          <div
            key={name}
            className="flex flex-col items-center gap-2 rounded-lg border border-border bg-surface p-3 text-center"
          >
            <Icon aria-hidden className="h-6 w-6 text-text" strokeWidth={2} />
            <span className="break-all text-mono-micro leading-tight text-text-dim">
              {name}
            </span>
          </div>
        ))}
      </div>
    </div>
  ),
};

/** Key icons in the command-deck medallion treatment — proof they read at
 *  medallion scale, lit (accent + glow) and quiet (muted). */
export const Medallions: Story = {
  render: () => {
    const picks: [string, IconCmp][] = (
      [
        "Boxes",
        "Sparkles",
        "FileCode2",
        "Waypoints",
        "Gauge",
        "ShieldCheck",
        "Terminal",
        "GitFork",
      ] as const
    ).map((n) => [n, (Icons as Record<string, IconCmp>)[n]]);
    return (
      <div className="min-h-screen bg-surface-sunken p-8">
        <div className="flex flex-wrap gap-6">
          {picks.map(([name, Icon], i) => (
            <div key={name} className="flex flex-col items-center gap-2">
              <span
                aria-hidden
                className={
                  i % 2 === 0
                    ? "relative grid h-12 w-12 place-items-center rounded-xl border border-accent/50 bg-surface-overlay text-accent shadow-[var(--glow-soft)]"
                    : "relative grid h-12 w-12 place-items-center rounded-xl border border-border bg-surface-overlay text-text-muted"
                }
              >
                <Icon className="h-5 w-5" strokeWidth={2} />
              </span>
              <span className="text-mono-micro text-text-dim">{name}</span>
            </div>
          ))}
        </div>
      </div>
    );
  },
};

/** The hairline holds its character across the real on-screen sizes. */
export const Sizes: Story = {
  render: () => {
    const { Waypoints } = Icons as Record<string, IconCmp>;
    return (
      <div className="flex min-h-screen items-end gap-6 bg-surface-sunken p-8">
        {[12, 14, 16, 18, 24, 32].map((px) => (
          <div key={px} className="flex flex-col items-center gap-2">
            <Waypoints
              aria-hidden
              className="text-text"
              style={{ width: px, height: px }}
              strokeWidth={2}
            />
            <span className="text-mono-micro text-text-dim">{px}px</span>
          </div>
        ))}
      </div>
    );
  },
};
