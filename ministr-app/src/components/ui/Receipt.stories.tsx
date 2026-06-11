import type { Meta, StoryObj } from "@storybook/react-vite";
import { Receipt } from "./Receipt";

const meta = {
  title: "Atoms/Receipt",
  component: Receipt,
} satisfies Meta<typeof Receipt>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Feed: Story = {
  args: { time: "10:43", sentence: "—" },
  render: () => (
    <div className="max-w-xl rounded-lg border border-line bg-surface p-1">
      <Receipt
        time="10:43"
        kind="win"
        sentence="your AI asked about “login button” → sent straight to LoginForm.tsx (1 search)"
      />
      <Receipt
        time="10:41"
        sentence="your AI needed handleSubmit() → exact definition + its 3 callers"
      />
      <Receipt
        time="10:39"
        kind="headsup"
        sentence="you changed lib/auth.ts — your AI read the old version at 10:40"
      />
    </div>
  ),
};
