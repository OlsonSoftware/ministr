import type { Meta, StoryObj } from "@storybook/react-vite";
import { ProjectCard } from "./ProjectCard";
import type { ProjectCardData } from "./ProjectCard";
import type { DerivedProgress } from "../../lib/progress";

const meta = {
  title: "Manager/ProjectCard",
  component: ProjectCard,
  decorators: [
    (Story) => (
      <div className="w-[44rem] max-w-full">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ProjectCard>;

export default meta;
type Story = StoryObj<typeof meta>;

const noop = () => {};
const actions = { onOpen: noop, onReindex: noop, onConfigure: noop, onRemove: noop };

const INDEXING_PROGRESS: DerivedProgress = {
  running: true,
  complete: false,
  phase: "embedding",
  percent: 0.62,
  ratePerSec: 124,
  stalled: false,
  etaSeconds: 48,
  currentFile: "src/services/auth.ts",
} as DerivedProgress;

/** Healthy index — calm: status rail green, the stats just sit there. */
export const Current: Story = {
  args: {
    data: {
      name: "ministr",
      status: "ok",
      files: 1482,
      sections: 6640,
      symbols: 1840,
      indexedAgo: "3m ago",
      stack: ["rust", "typescript", "go", "python"],
    } satisfies ProjectCardData,
    ...actions,
  },
};

/** Behind your changes — the amber rail + "2 behind" chip do the work a
 *  prose sentence used to; reindex is one icon away. */
export const Behind: Story = {
  args: {
    data: {
      name: "side-project",
      status: "stale",
      files: 312,
      sections: 1290,
      symbols: 410,
      indexedAgo: "2h ago",
      behind: 2,
      stack: ["typescript", "rust"],
    } satisfies ProjectCardData,
    ...actions,
  },
};

/** Indexing now — the instrument replaces the stat strip; brand rail. */
export const Indexing: Story = {
  args: {
    data: {
      name: "my-app",
      status: "updating",
      files: 1482,
      sections: 0,
      stack: ["typescript", "go"],
      progress: INDEXING_PROGRESS,
    } satisfies ProjectCardData,
    ...actions,
  },
};

/** A live agent is reading this index right now (lite presence: pulse + count). */
export const LiveAgent: Story = {
  args: {
    data: {
      name: "ministr-private",
      status: "ok",
      files: 904,
      sections: 4120,
      symbols: 1120,
      indexedAgo: "just now",
      agents: 2,
      stack: ["rust", "python"],
    } satisfies ProjectCardData,
    ...actions,
  },
};

/** The manager list — several indexes at a glance (the new Home body). */
export const ManagerList: Story = {
  args: { data: Current.args!.data as ProjectCardData, ...actions },
  render: (args) => (
    <div className="flex flex-col gap-3">
      <ProjectCard {...args} data={Behind.args!.data as ProjectCardData} />
      <ProjectCard {...args} data={Indexing.args!.data as ProjectCardData} />
      <ProjectCard {...args} data={LiveAgent.args!.data as ProjectCardData} />
      <ProjectCard {...args} data={Current.args!.data as ProjectCardData} />
    </div>
  ),
};
