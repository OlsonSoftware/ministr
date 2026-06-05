import type { Meta, StoryObj } from "@storybook/react-vite";
import { Logo, Wordmark } from "./logo";

/** The ministr brand mark + wordmark lockup (from `brand/logo.svg`). */
const meta = {
  title: "UI/Logo",
  component: Logo,
  parameters: { layout: "centered" },
} satisfies Meta<typeof Logo>;

export default meta;
type Story = StoryObj<typeof meta>;

/** The mark at a range of sizes — full brand gradient. */
export const Mark: Story = {
  render: () => (
    <div className="flex items-end gap-6 p-6">
      {[16, 24, 32, 48, 64].map((px) => (
        <div key={px} className="flex flex-col items-center gap-2">
          <Logo style={{ width: px, height: px }} title="ministr" />
          <span className="text-mono-micro text-text-dim">{px}px</span>
        </div>
      ))}
    </div>
  ),
};

/** Mono mode — `currentColor`, so the mark tones with its container
 *  (accent, then danger, as a command-deck medallion would). */
export const Mono: Story = {
  render: () => (
    <div className="flex items-center gap-6 p-6">
      <Logo gradient={false} className="h-10 w-10 text-text" title="ministr" />
      <Logo gradient={false} className="h-10 w-10 text-accent" title="ministr" />
      <Logo gradient={false} className="h-10 w-10 text-danger" title="ministr" />
    </div>
  ),
};

/** The wordmark lockup — mark + `ministr`, as it appears in the top chrome. */
export const WordmarkLockup: Story = {
  render: () => (
    <div className="flex flex-col items-start gap-5 p-6 text-2xl">
      <Wordmark />
      <span className="text-base">
        <Wordmark />
      </span>
    </div>
  ),
};

/** The boot-hero medallion treatment — the mark in a glowing command-deck frame. */
export const BootMedallion: Story = {
  render: () => (
    <div className="flex gap-8 p-8">
      <span className="grid h-16 w-16 place-items-center rounded-2xl border border-accent/50 bg-surface-overlay text-accent shadow-[var(--glow-soft)]">
        <Logo className="h-7 w-7" title="ministr" />
      </span>
      <span className="grid h-16 w-16 place-items-center rounded-2xl border border-danger/50 bg-surface-overlay text-danger">
        <Logo gradient={false} className="h-7 w-7" title="ministr (offline)" />
      </span>
    </div>
  ),
};
