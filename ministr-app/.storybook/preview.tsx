import type { Preview } from "@storybook/react-vite";
import { withThemeByClassName } from "@storybook/addon-themes";
import "../src/app.css";

/**
 * Loads the real app.css (tokens, glass, motion, reduced-motion) so stories
 * render on the true design contract. The `.dark` class toggle mirrors the
 * app's theming; every primitive should be reviewed in both light and dark.
 */
const preview: Preview = {
  parameters: {
    controls: { matchers: { color: /(background|color)$/i, date: /Date$/i } },
    // axe runs on every story. "error" makes a WCAG violation FAIL the Vitest
    // run (via addon-vitest), so the §9 floor is enforced mechanically in the
    // gate — not just surfaced in the interactive a11y panel.
    a11y: { test: "error" },
  },
  decorators: [
    withThemeByClassName({
      themes: { light: "", dark: "dark" },
      defaultTheme: "dark",
    }),
    (Story) => (
      <div className="min-h-screen bg-bg text-text p-10">
        <Story />
      </div>
    ),
  ],
};

export default preview;
