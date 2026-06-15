import type { Meta, StoryObj } from "@storybook/react-vite";
import { StatusBanner } from "./StatusBanner";
import { ActionChip } from "./ActionChip";

const meta = {
  title: "Atoms/StatusBanner",
  component: StatusBanner,
} satisfies Meta<typeof StatusBanner>;

export default meta;
type Story = StoryObj<typeof meta>;

export const UpToDate: Story = {
  args: {
    state: "ok",
    headline: "Your AI sees your code — up to date",
    sub: "last change picked up 40 seconds ago · 1 agent reading",
  },
};

export const Behind: Story = {
  args: {
    state: "stale",
    headline: "Your AI is 3 saves behind",
    sub: "it may answer from old code (mostly login.tsx)",
    action: <ActionChip variant="primary">Catch up · ~40s</ActionChip>,
  },
};

export const Updating: Story = {
  args: {
    state: "updating",
    headline: "Catching up…",
    sub: "reading the 3 files you changed",
  },
};

export const Hidden: Story = {
  args: {
    state: "hidden",
    headline: "Hidden from your AI",
    sub: "this folder is excluded by an ignore rule",
  },
};

/** The whole point of C5: three states side by side read pre-attentively
 *  — the behind card tints + rails, hidden recedes, healthy stays quiet. */
export const TrustCueStack: Story = {
  args: { state: "stale", headline: "" },
  render: () => (
    <div className="flex w-[28rem] flex-col gap-3">
      <StatusBanner
        state="stale"
        headline="Your AI is 3 saves behind"
        sub="side-project · it may answer from old code"
      />
      <StatusBanner
        state="ok"
        headline="Your AI sees your code — up to date"
        sub="my-app · everything matches your working tree"
      />
      <StatusBanner
        state="hidden"
        headline="Hidden from your AI"
        sub="scratch · excluded by an ignore rule"
      />
    </div>
  ),
};
