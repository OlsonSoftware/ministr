import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CorpusInfo } from "../../lib/types";
import { CorpusSelect } from "./corpus-select";

const CORPORA = [
  { id: "ministr", root: "/Users/alrik/Code/ministr" },
  { id: "ministr-private", root: "/Users/alrik/Code/ministr-private" },
  { id: "ministr-planning", root: "/Users/alrik/Code/ministr-planning" },
] as unknown as CorpusInfo[];

const meta = {
  title: "UI/CorpusSelect",
  component: CorpusSelect,
  args: { value: "ministr", corpora: CORPORA, onChange: () => {} },
} satisfies Meta<typeof CorpusSelect>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: (args) => {
    const [value, setValue] = useState(args.value);
    return <CorpusSelect {...args} value={value} onChange={setValue} />;
  },
};

export const Disabled: Story = { args: { disabled: true } };
