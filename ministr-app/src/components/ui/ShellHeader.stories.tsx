import type { Meta, StoryObj } from "@storybook/react-vite";
import { ShellHeader } from "./ShellHeader";
import { Brand } from "./Brand";
import { BackButton } from "./BackButton";
import { ActionChip } from "./ActionChip";
import { SettingsMenu } from "./SettingsMenu";
import { withTauriMock } from "../../../.storybook/tauri-mock";

const meta = {
  title: "Atoms/ShellHeader",
  component: ShellHeader,
  decorators: [
    (Story) => (
      <div className="w-[64rem] max-w-full">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ShellHeader>;

export default meta;
type Story = StoryObj<typeof meta>;

const DAEMON = {
  version: "0.7.0",
  uptime_secs: 12_240,
  memory_mb: 184.6,
  model: "bge-small-en-v1.5",
  model_dimension: 384,
  corpora: [],
  total_sessions: 7,
};

/** Home/root shape — identity on the left, global actions right. The same
 *  top every root screen now renders. */
export const RootIdentity: Story = {
  args: {
    leading: <Brand />,
    trailing: <SettingsMenu />,
  },
  decorators: [withTauriMock({ daemon_status: DAEMON })],
};

/** Drill-in shape — back affordance + titled object, with a per-screen
 *  trailing action. Mirror uses exactly this. */
export const DrillInWithAction: Story = {
  args: {
    leading: <BackButton onClick={() => {}} label="All projects" />,
    title: "side-project",
    subtitle: "what your AI sees",
    trailing: <ActionChip onClick={() => {}}>What ministr did</ActionChip>,
  },
};

/** Drill-in, title only — the Feed shape (no trailing action). */
export const DrillInTitleOnly: Story = {
  args: {
    leading: <BackButton onClick={() => {}} label="All projects" />,
    title: "side-project",
    subtitle: "what ministr did for your AI",
  },
};

/** Identity only — the Connect/welcome shape: Brand top-left, nothing else. */
export const IdentityOnly: Story = {
  args: {
    leading: <Brand />,
  },
};
