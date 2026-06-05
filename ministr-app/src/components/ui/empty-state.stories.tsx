import type { Meta, StoryObj } from "@storybook/react-vite";
import { FolderOpen } from "@/components/ui/icons";
import { EmptyState } from "./empty-state";
import { Button } from "./button";

const meta = {
  title: "UI/EmptyState",
  component: EmptyState,
  args: {
    icon: FolderOpen,
    title: "No projects yet",
    hint: "Index a repository to start exploring it with ministr.",
  },
} satisfies Meta<typeof EmptyState>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Muted: Story = {};

export const AccentCTA: Story = {
  args: {
    accent: true,
    action: <Button size="sm">Index a repo</Button>,
  },
};
