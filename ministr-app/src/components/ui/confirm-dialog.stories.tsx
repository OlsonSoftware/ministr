import type { Meta, StoryObj } from "@storybook/react-vite";
import { ConfirmDialog } from "./confirm-dialog";

const meta = {
  title: "UI/ConfirmDialog",
  component: ConfirmDialog,
  parameters: { layout: "fullscreen" },
  args: {
    open: true,
    title: "Delete corpus",
    body: "This removes the index and all derived data. This cannot be undone.",
    onCancel: () => {},
    onConfirm: () => {},
  },
} satisfies Meta<typeof ConfirmDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Danger: Story = {
  args: { tone: "danger", confirmLabel: "Delete" },
};

export const TypeToConfirm: Story = {
  args: {
    tone: "danger",
    confirmLabel: "Delete",
    confirmToken: "ministr",
    body: "Type the corpus name to confirm deletion.",
  },
};

/**
 * Renders the dialog over a vivid, varied backdrop so the §4 glass tier is
 * actually visible — the panel should frost + tint the colored shapes behind
 * it (backdrop-blur + translucent surface), not sit on a flat fill. Toggle the
 * OS "Reduce Transparency" setting and this same story should collapse to a
 * solid surface (the mandatory a11y fallback).
 */
export const OverContent: Story = {
  args: { tone: "default" },
  render: (args) => (
    <div className="relative h-screen w-full overflow-hidden">
      <div className="absolute inset-0 grid grid-cols-3 gap-4 p-8">
        <div className="h-40 rounded-xl bg-accent" />
        <div className="h-56 rounded-xl bg-success" />
        <div className="h-32 rounded-xl bg-info" />
        <div className="h-48 rounded-xl bg-warning" />
        <div className="h-36 rounded-xl bg-danger" />
        <div className="h-52 rounded-xl bg-accent" />
        <div className="col-span-3 h-28 rounded-xl bg-gradient-to-r from-accent via-info to-success" />
      </div>
      <ConfirmDialog {...args} />
    </div>
  ),
};
