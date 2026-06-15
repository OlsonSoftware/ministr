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
      <TreeRow name="src/" state="ok" disclosure="expanded" />
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

/**
 * Affordance — the row announces its interactive role: a leading caret on
 * directories (▸ collapsed / ▾ open) and a quiet trailing chevron on files
 * that open a drill-in. Wrap in a real <button> to see the hover reveal.
 */
export const Clickable: Story = {
  args: { name: "src/", state: "ok" },
  render: () => (
    <div className="max-w-xl rounded-lg border border-line bg-surface p-1">
      <button type="button" className="block w-full cursor-pointer text-left">
        <TreeRow name="src/" state="ok" disclosure="expandable" />
      </button>
      <button type="button" className="block w-full cursor-pointer text-left">
        <TreeRow name="components/" state="stale" disclosure="expanded" />
      </button>
      <button type="button" className="block w-full cursor-pointer text-left">
        <TreeRow
          name="components/LoginForm.tsx"
          state="ok"
          level={1}
          disclosure="navigates"
        />
      </button>
      <button type="button" className="block w-full cursor-pointer text-left">
        <TreeRow
          name="lib/auth.ts"
          state="stale"
          level={1}
          note="you changed this 5 minutes ago"
          disclosure="navigates"
        />
      </button>
    </div>
  ),
};
