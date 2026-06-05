import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { SystemVitalsPulse } from "./SystemVitalsPulse";

/**
 * SystemVitalsPulse — the daemon's memory footprint as a living trend, the
 * inset that lives on the System Diagnostics deck. All states render from a
 * static `series` prop (MB, oldest → newest), so the stories are deterministic
 * and axe gates the surface on every run. Framed over the raised deck tier so
 * the inset border reads as it does in the app.
 */

// Deterministic series builders — no Date.now()/Math.random() (gate-stable).
const STEADY = Array.from({ length: 40 }, (_, i) =>
  142 + 4 * Math.sin(i / 3) + (i % 5 === 0 ? 2 : 0),
);
const RAMP = Array.from({ length: 40 }, (_, i) => 118 + i * 2.4);
const SAWTOOTH = Array.from({ length: 44 }, (_, i) => {
  const cycle = i % 11; // climb 0..10 then drop (GC)
  return 130 + cycle * 7;
});
const WARMING = [148];

function Frame({ children }: { children: ReactNode }) {
  return (
    <div className="max-w-md bg-surface-raised p-4">
      {/* Mimic the Diagnostics deck the pulse sits inside. */}
      {children}
    </div>
  );
}

const meta = {
  title: "Surfaces/SystemVitalsPulse",
  component: SystemVitalsPulse,
  parameters: { layout: "padded" },
  decorators: [
    (Story) => (
      <Frame>
        <Story />
      </Frame>
    ),
  ],
} satisfies Meta<typeof SystemVitalsPulse>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Steady — memory breathes around a flat baseline. The healthy daemon: now ≈
 *  peak, Δ near zero. */
export const Steady: Story = { args: { series: STEADY } };

/** Climbing — a monotonic ramp (a leak): Δ window is a big positive, peak = now. */
export const Climbing: Story = { args: { series: RAMP } };

/** GC sawtooth — repeated climb-then-collect; the trend's signature shape. */
export const Sawtooth: Story = { args: { series: SAWTOOTH } };

/** Warming — fewer than two samples yet; the inset shows the collecting state
 *  instead of a one-point chart. */
export const Warming: Story = { args: { series: WARMING } };
