import type { Meta, StoryObj } from "@storybook/react-vite";
import { ErrorCallout } from "./error-callout";
import { Button } from "./button";

const meta = {
  title: "UI/ErrorCallout",
  component: ErrorCallout,
  args: {
    title: "Indexing failed",
    message: "daemon error: corpus 'ministr' not found (is the daemon running?)",
  },
} satisfies Meta<typeof ErrorCallout>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const WithAction: Story = {
  args: {
    action: (
      <Button variant="outline" size="sm">
        Retry
      </Button>
    ),
  },
};

export const MessageOnly: Story = {
  args: { title: undefined, message: "Connection refused on /var/run/ministr.sock" },
};
