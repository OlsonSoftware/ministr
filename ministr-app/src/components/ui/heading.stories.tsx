import type { Meta, StoryObj } from "@storybook/react-vite";
import { H1, H2, H3 } from "./heading";

const meta = {
  title: "UI/Heading",
  component: H1,
  args: { children: null },
} satisfies Meta<typeof H1>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Scale: Story = {
  render: () => (
    <div className="flex flex-col gap-4">
      <H1>Projects</H1>
      <H2>Retrieval settings</H2>
      <H3>Per-corpus config</H3>
      <p className="font-sans text-sm text-text-muted leading-relaxed max-w-md">
        Body prose pairs below a heading — Geist sans, secondary contrast.
      </p>
    </div>
  ),
};
