import type { Meta, StoryObj } from "@storybook/react-vite";
import { BackButton } from "./BackButton";

const meta = {
  title: "Atoms/BackButton",
  component: BackButton,
} satisfies Meta<typeof BackButton>;

export default meta;
type Story = StoryObj<typeof meta>;

/** The Mirror's back: returns to the project list. */
export const AllProjects: Story = {
  args: { onClick: () => {}, label: "All projects" },
};

/** The Feed's back when opened from a project: returns to that project. */
export const ToProject: Story = {
  args: { onClick: () => {}, label: "my-app" },
};
