import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { FilterPill } from "./filter-pill";

const meta = {
  title: "UI/FilterPill",
  component: FilterPill,
  args: { label: "Rust", active: false, onClick: () => {} },
} satisfies Meta<typeof FilterPill>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Row: Story = {
  render: () => {
    const [active, setActive] = useState("all");
    const opts = [
      { id: "all", label: "All", count: 312 },
      { id: "rust", label: "Rust", count: 184 },
      { id: "ts", label: "TypeScript", count: 96 },
      { id: "py", label: "Python", count: 32 },
    ];
    return (
      <div className="flex flex-wrap items-center gap-2">
        {opts.map((o) => (
          <FilterPill
            key={o.id}
            label={o.label}
            count={o.count}
            active={active === o.id}
            onClick={() => setActive(o.id)}
          />
        ))}
      </div>
    );
  },
};
