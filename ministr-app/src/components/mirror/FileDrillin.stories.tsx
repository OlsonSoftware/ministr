import type { Meta, StoryObj } from "@storybook/react-vite";
import { FileDrillin } from "./FileDrillin";
import { withTauriMock } from "../../../.storybook/tauri-mock";

const SECTIONS = {
  found: true,
  sections: [
    {
      heading: "auth › handleSubmit",
      text: "export async function handleSubmit(form: LoginForm) {\n  const session = await login(form.email, form.password);\n  return session;\n}",
    },
    {
      heading: "auth › validate",
      text: "function validate(form: LoginForm): boolean {\n  return form.email.includes(\"@\");\n}",
    },
  ],
};

const meta = {
  title: "Screens/FileDrillin",
  component: FileDrillin,
  decorators: [
    withTauriMock({
      indexed_file: SECTIONS,
      read_file: {
        content:
          "export async function handleSubmit(form: LoginForm) {\n  // NEW: rate limiting added after the index last read this\n  await rateLimit(form.email);\n  const session = await login(form.email, form.password);\n  return session;\n}",
        symbols: [],
      },
    }),
  ],
} satisfies Meta<typeof FileDrillin>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Current: Story = {
  args: {
    corpusId: "corpus-bbbb",
    path: "src/lib/auth.ts",
    state: "ok",
    onBack: () => {},
  },
};

export const Stale: Story = {
  args: {
    corpusId: "corpus-bbbb",
    path: "src/lib/auth.ts",
    state: "stale",
    onBack: () => {},
  },
};

export const NeverIndexed: Story = {
  args: {
    corpusId: "corpus-bbbb",
    path: "src/new-file.ts",
    state: "stale",
    onBack: () => {},
  },
  decorators: [
    withTauriMock({
      indexed_file: { found: false, sections: [] },
    }),
  ],
};
