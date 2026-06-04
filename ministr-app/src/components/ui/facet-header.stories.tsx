import type { Meta, StoryObj } from "@storybook/react-vite";
import { Activity, Compass } from "lucide-react";
import { FacetHeader } from "./facet-header";
import { Button } from "./button";

/**
 * FacetHeader — the shared facet title row (icon? + title + glance + actions +
 * optional sub-content). The cohesion grammar adopted by Activity and Fleet so
 * the workspace facets share one identity row (aaa-views-cohesion-sweep).
 */
const meta = {
  title: "UI/FacetHeader",
  component: FacetHeader,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof FacetHeader>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Title + glance only — the minimal facet identity row. */
export const TitleAndGlance: Story = {
  args: {
    title: "Sessions",
    glance: "4 live agent sessions.",
  },
};

/** With an identity icon. */
export const WithIcon: Story = {
  args: {
    icon: Compass,
    title: "Explore",
    glance: "1,284 files · 18,422 symbols",
  },
};

/** With right-aligned actions (the Fleet pattern). */
export const WithActions: Story = {
  args: {
    icon: Activity,
    title: "Fleet",
    glance: (
      <>
        5 projects · <span className="text-accent">4 live</span>
      </>
    ),
    actions: (
      <>
        <Button variant="outline" size="sm">
          Scan
        </Button>
        <Button size="sm">Add project</Button>
      </>
    ),
  },
};

/** With a sub-content block under the row (e.g. a vitals tile grid). */
export const WithChildren: Story = {
  args: {
    title: "Fleet",
    glance: "5 projects · 4 live",
    children: (
      <div className="grid grid-cols-2 gap-px overflow-hidden rounded-lg border border-border bg-border sm:grid-cols-4">
        {["Files", "Vectors", "Symbols", "Live agents"].map((l) => (
          <div key={l} className="bg-surface p-3">
            <p className="font-mono text-mono-micro uppercase tracking-wide text-text-dim">
              {l}
            </p>
            <p className="font-mono text-base text-text">—</p>
          </div>
        ))}
      </div>
    ),
  },
};
