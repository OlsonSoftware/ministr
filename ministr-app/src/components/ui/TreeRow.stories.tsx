import type { Meta, StoryObj } from "@storybook/react-vite";
import { TreeRow } from "./TreeRow";
import { ActionChip } from "./ActionChip";

const meta = {
  title: "Atoms/TreeRow",
  component: TreeRow,
} satisfies Meta<typeof TreeRow>;

export default meta;
type Story = StoryObj<typeof meta>;

export const MirrorSlice: Story = {
  args: { name: "src/", state: "ok" },
  render: () => (
    <div className="max-w-xl rounded-lg border border-line bg-surface p-1">
      <TreeRow name="src/" state="ok" />
      <TreeRow name="components/LoginForm.tsx" state="ok" level={1} />
      <TreeRow name="components/Navbar.tsx" state="ok" level={1} />
      <TreeRow
        name="lib/auth.ts"
        state="stale"
        level={1}
        note="you changed this 5 minutes ago"
        action={<ActionChip>Update now</ActionChip>}
      />
      <TreeRow name="lib/db.ts" state="updating" level={1} note="updating…" />
      <TreeRow
        name="secrets.json"
        state="hidden"
        level={1}
        note="you excluded it"
      />
    </div>
  ),
};
