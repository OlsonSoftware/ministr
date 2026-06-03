import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Toggle, ToggleRow } from "./toggle";

const meta = {
  title: "UI/Toggle",
  component: Toggle,
  args: { enabled: false, onToggle: () => {} },
} satisfies Meta<typeof Toggle>;

export default meta;
type Story = StoryObj<typeof meta>;

export const States: Story = {
  render: () => {
    const [on, setOn] = useState(true);
    return (
      <div className="flex items-center gap-4">
        <Toggle enabled={on} onToggle={() => setOn((v) => !v)} ariaLabel="demo" />
        <Toggle enabled={false} onToggle={() => {}} ariaLabel="off" />
        <Toggle enabled={null} onToggle={() => {}} ariaLabel="pending" />
      </div>
    );
  },
};

export const Row: Story = {
  render: () => {
    const [on, setOn] = useState(false);
    return (
      <div className="w-80">
        <ToggleRow
          label="Hybrid retrieval"
          description="Blend dense + sparse vectors at query time"
          enabled={on}
          onToggle={() => setOn((v) => !v)}
        />
      </div>
    );
  },
};
