import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Cloud, KeyRound, Plug, Webhook } from "@/components/ui/icons";
import { SurfaceSidebar, type SidebarItem } from "./surface-sidebar";

const ITEMS: SidebarItem[] = [
  { id: "connection", label: "Connection", icon: Plug },
  { id: "corpora", label: "Corpora", icon: Cloud },
  { id: "keys", label: "API Keys", icon: KeyRound },
  { id: "webhooks", label: "Webhooks", icon: Webhook },
];

const meta = {
  title: "UI/SurfaceSidebar",
  component: SurfaceSidebar,
  parameters: { layout: "fullscreen" },
  args: { title: "Cloud", items: ITEMS, active: "connection", onSelect: () => {}, children: null },
} satisfies Meta<typeof SurfaceSidebar>;

export default meta;
type Story = StoryObj<typeof meta>;

export const MasterDetail: Story = {
  render: () => {
    const [active, setActive] = useState("corpora");
    return (
      <div className="h-[26rem] w-full">
        <SurfaceSidebar title="Cloud" items={ITEMS} active={active} onSelect={setActive}>
          <div className="flex flex-col gap-2">
            <h2 className="font-sans text-base font-semibold text-text">
              {ITEMS.find((i) => i.id === active)?.label}
            </h2>
            <p className="text-sm text-text-muted">
              Section content for “{active}”. The nav rail is wide-viewport; it
              collapses to a top tab bar below the @900px container width.
            </p>
          </div>
        </SurfaceSidebar>
      </div>
    );
  },
};
