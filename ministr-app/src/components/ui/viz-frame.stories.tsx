import type { Meta, StoryObj } from "@storybook/react-vite";
import { Activity, GitCompareArrows } from "@/components/ui/icons";
import { VizFrame } from "./viz-frame";

/**
 * VizFrame — the shared command-deck panel the data-viz suite renders in
 * (aaa-viz-frame-cohesion): raised tier + accent lit-edge + optional eyebrow +
 * optional readout + the chart as children. Adopted by ActivityPulse,
 * DiffRipple and CodebaseConstellation so the suite reads as one family.
 */
const meta = {
  title: "UI/VizFrame",
  component: VizFrame,
  parameters: { layout: "padded" },
  decorators: [
    (Story) => (
      <div className="w-full max-w-[560px] bg-surface p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof VizFrame>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A stand-in chart surface for the catalog. */
function Chart() {
  return (
    <div className="grid h-24 place-items-center rounded-md border border-border-soft bg-surface-sunken/40 font-mono text-mono-micro text-text-dim">
      chart
    </div>
  );
}

const readout = (
  <>
    <span className="flex items-center gap-1">
      <span className="tabular-nums font-semibold text-text">128</span> calls
    </span>
    <span aria-hidden className="text-border">·</span>
    <span className="flex items-center gap-1 text-success">
      <span className="tabular-nums font-semibold">92%</span> cache
    </span>
  </>
);

/** Eyebrow + readout — the ActivityPulse / DiffRipple shape. */
export const WithEyebrowAndReadout: Story = {
  args: { icon: Activity, label: "Live activity", readout, children: <Chart /> },
};

/** Eyebrow only — a labelled panel with no metrics. */
export const EyebrowOnly: Story = {
  args: { icon: GitCompareArrows, label: "Blast ripple", children: <Chart /> },
};

/** Readout only (no eyebrow) — the CodebaseConstellation shape, where a host
 *  SectionLabel provides the title and the readout self-justifies. */
export const ReadoutOnly: Story = {
  args: {
    readout: (
      <>
        <span>
          <span className="tabular-nums font-semibold text-text">10</span> modules ·{" "}
          <span className="tabular-nums font-semibold text-text">149</span> files
        </span>
        <span>sized by index mass</span>
      </>
    ),
    children: <Chart />,
  },
};

/** Bare frame — just the lit-edge panel around a chart. */
export const Bare: Story = { args: { children: <Chart /> } };
