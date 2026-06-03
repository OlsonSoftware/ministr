import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  BrutalSearch,
  BrutalExplore,
  BrutalSymbols,
  BrutalBridge,
  BrutalProjects,
  BrutalStructure,
  BrutalSessions,
  BrutalLogs,
  BrutalAsk,
  BrutalSettings,
} from "./brutal-icons";

const ICONS = [
  ["Search", BrutalSearch],
  ["Explore", BrutalExplore],
  ["Symbols", BrutalSymbols],
  ["Bridge", BrutalBridge],
  ["Projects", BrutalProjects],
  ["Structure", BrutalStructure],
  ["Sessions", BrutalSessions],
  ["Logs", BrutalLogs],
  ["Ask", BrutalAsk],
  ["Settings", BrutalSettings],
] as const;

const meta = { title: "UI/BrutalIcons" } satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

export const Gallery: Story = {
  render: () => (
    <div className="flex flex-wrap gap-5">
      {ICONS.map(([name, Icon]) => (
        <div key={name} className="flex flex-col items-center gap-1.5">
          <Icon className="h-7 w-7 text-text" />
          <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
            {name}
          </span>
        </div>
      ))}
    </div>
  ),
};
