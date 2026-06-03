import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Chip, ChipGroup } from "./chip-group";

const meta = {
  title: "UI/ChipGroup",
  component: ChipGroup,
  args: { children: null },
} satisfies Meta<typeof ChipGroup>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Filters: Story = {
  render: () => {
    const [active, setActive] = useState("survey");
    const tools = [
      { id: "survey", label: "survey", count: 142 },
      { id: "read", label: "read", count: 88 },
      { id: "symbols", label: "symbols", count: 54 },
      { id: "refs", label: "references", count: 21 },
    ];
    return (
      <ChipGroup>
        {tools.map((t) => (
          <Chip
            key={t.id}
            label={t.label}
            count={t.count}
            active={active === t.id}
            onClick={() => setActive(t.id)}
          />
        ))}
      </ChipGroup>
    );
  },
};
