import type { StorybookConfig } from "@storybook/react-vite";

/**
 * Storybook — the AAA visual-iteration substrate (DESIGN.md §11.B).
 * Reuses the app's Vite config (Tailwind v4 + tokens), so primitives render
 * exactly as in the app, in isolation, with no Tauri runtime. Browser-based,
 * so Playwright MCP can drive it for before/after screenshots + a11y checks.
 */
const config: StorybookConfig = {
  stories: ["../src/components/**/*.stories.@(ts|tsx)"],
  addons: [
    // in-browser axe — mechanizes part of the §9 WCAG DoD
    "@storybook/addon-a11y",
    // light/dark via the .dark class (matches app.css)
    "@storybook/addon-themes",
  ],
  framework: { name: "@storybook/react-vite", options: {} },
};

export default config;
