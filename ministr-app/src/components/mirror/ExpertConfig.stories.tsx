import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import { ExpertConfig } from "./ExpertConfig";
import { withTauriMock } from "../../../.storybook/tauri-mock";

const MODELS = [
  {
    name: "minilm",
    dimension: 384,
    description: "fast general model",
    code_optimized: false,
  },
  {
    name: "jina-code",
    dimension: 768,
    description: "code-optimised",
    code_optimized: true,
  },
];

const meta = {
  title: "Screens/ProjectMirror/ExpertConfig",
  component: ExpertConfig,
  decorators: [
    withTauriMock({
      list_supported_models: MODELS,
      set_corpus_config: null,
    }),
  ],
} satisfies Meta<typeof ExpertConfig>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Collapsed by default — internals vocabulary stays behind the fold. */
export const Collapsed: Story = {
  args: { corpusId: "c1", model: "minilm" },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await canvas.findByText(/how ministr reads this project · expert/);
    // a closed <details> keeps children in the DOM — the disclosure
    // contract is the missing `open` attribute, not absence
    const details = canvasElement.querySelector("details");
    await expect(details?.hasAttribute("open")).toBe(false);
  },
};

/** Open → pick the code model → Save fires set_corpus_config and the
 *  onSaved callback (the Mirror's optimistic Catching-up hook). */
export const SaveFlow: Story = {
  args: { corpusId: "c1", model: "minilm" },
  play: async ({ canvasElement, userEvent }) => {
    const canvas = within(canvasElement);
    await userEvent.click(
      await canvas.findByText(/how ministr reads this project/),
    );
    const select = await canvas.findByLabelText(/embedding model/);
    await userEvent.selectOptions(select, "jina-code");
    const save = await canvas.findByRole("button", { name: /save & re-read/i });
    await userEvent.click(save);
    // success returns the chip to its idle label (no failure copy)
    await canvas.findByRole("button", { name: /save & re-read/i });
    await expect(canvas.queryByText(/couldn.t save/i)).toBeNull();
  },
};
