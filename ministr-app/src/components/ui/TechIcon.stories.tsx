import type { Meta, StoryObj } from "@storybook/react-vite";
import { TechIcon } from "./TechIcon";

const meta = {
  title: "Atoms/TechIcon",
  component: TechIcon,
} satisfies Meta<typeof TechIcon>;

export default meta;
type Story = StoryObj<typeof meta>;

const ALL = [
  "rust",
  "typescript",
  "javascript",
  "python",
  "go",
  "java",
  "kotlin",
  "php",
  "ruby",
  "csharp",
  "swift",
  "scala",
  "cpp",
  "elixir",
];

/** Every detected tech, neutral at rest. Hover one to see its real brand
 *  colour (the sanctioned interaction reward). */
export const Gallery: Story = {
  args: { slug: "rust" },
  render: () => (
    <div className="flex flex-wrap items-center gap-4">
      {ALL.map((slug) => (
        <TechIcon key={slug} slug={slug} />
      ))}
    </div>
  ),
};

/** An unknown slug renders nothing (no broken box). */
export const UnknownIsSkipped: Story = {
  args: { slug: "cobol" },
  render: () => (
    <div className="text-sm text-dim">
      [<TechIcon slug="cobol" />] ← nothing renders for an unknown tech
    </div>
  ),
};
