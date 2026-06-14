import type { Meta, StoryObj } from "@storybook/react-vite";
import { Screen } from "./Screen";
import { Brand } from "./Brand";
import { StatusBanner } from "./StatusBanner";
import { ActionChip } from "./ActionChip";

/**
 * The shared screen shell. These stories prove the two behaviours the four
 * screen roots used to get wrong: short content CENTERS (no top-aligned
 * void), tall content SCROLLS from the top — and every screen keeps a
 * persistent, neutral trust-footer pinned to the bottom.
 */
const meta = {
  title: "Screens/Screen",
  component: Screen,
} satisfies Meta<typeof Screen>;

export default meta;
type Story = StoryObj<typeof meta>;

const Card = ({ n }: { n: number }) => (
  <StatusBanner
    state="ok"
    headline={`my-project-${n}`}
    sub="up to date · your AI sees everything"
  />
);

/** Short content, centered — the ConnectFlow-style hero with no void.
 *  The Brand rides INSIDE the centered content (no header slot), so the
 *  whole hero column centers together — the reference's exact rhythm. */
export const ShortCentered: Story = {
  args: {
    width: "xl",
    align: "center",
    version: "0.6.0",
    children: (
      <>
        <Brand size="lg" />
        <h1 className="text-2xl font-semibold tracking-tight text-ink">
          Point ministr at your project
        </h1>
        <p className="text-sm text-dim">
          Pick the folder you code in. Everything stays on your computer.
        </p>
        <div>
          <ActionChip variant="primary">Choose a folder…</ActionChip>
        </div>
      </>
    ),
  },
};

/** A short list, top-aligned — the Trust Panel rhythm. */
export const ShortList: Story = {
  args: {
    align: "start",
    version: "0.6.0",
    header: <Brand />,
    children: (
      <>
        <Card n={1} />
        <Card n={2} />
      </>
    ),
  },
};

/** Many rows — content scrolls from the top; header + footer stay put. */
export const TallScrolls: Story = {
  args: {
    align: "center",
    version: "0.6.0",
    header: <Brand />,
    children: (
      <>
        {Array.from({ length: 14 }, (_, i) => (
          <Card key={i} n={i + 1} />
        ))}
      </>
    ),
  },
};

/** No header, default footer — the minimal shell. */
export const FooterOnly: Story = {
  args: {
    align: "center",
    version: "0.6.0",
    children: (
      <p className="text-center text-sm text-dim">
        the calm baseline: content centered, trust-footer pinned below
      </p>
    ),
  },
};
