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
