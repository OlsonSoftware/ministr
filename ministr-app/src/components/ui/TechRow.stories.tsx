import type { Meta, StoryObj } from "@storybook/react-vite";
import { TechRow } from "./TechRow";

const meta = {
  title: "Atoms/TechRow",
  component: TechRow,
} satisfies Meta<typeof TechRow>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A typical project's detected stack. Wrap in `group` to see the
 *  card-hover lighting (each icon → its real brand colour). */
export const Typical: Story = {
  args: { slugs: ["rust", "typescript", "go", "python"] },
  render: (args) => (
    <div className="group max-w-md rounded-lg border border-line bg-surface p-4">
      <TechRow {...args} />
    </div>
  ),
};

/** A polyglot repo collapses the tail into "+N". */
export const Overflow: Story = {
  args: {
    slugs: ["typescript", "ruby", "elixir", "swift", "kotlin", "scala", "cpp"],
    max: 6,
  },
  render: (args) => (
    <div className="group max-w-md rounded-lg border border-line bg-surface p-4">
      <TechRow {...args} />
    </div>
  ),
};

/** Unknown techs drop out; an all-unknown stack renders nothing. */
export const UnknownsDropped: Story = {
  args: { slugs: ["rust", "cobol", "fortran", "go"] },
  render: (args) => (
    <div className="group max-w-md rounded-lg border border-line bg-surface p-4">
      <TechRow {...args} />
    </div>
  ),
};
