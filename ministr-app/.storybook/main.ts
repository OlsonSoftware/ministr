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
    // in-browser axe — mechanizes the §9 WCAG DoD. Paired with addon-vitest +
    // `a11y: { test: "error" }` (preview.tsx), axe runs on EVERY story in CI and
    // a violation fails `pnpm test` — the floor is now mechanical, not manual.
    "@storybook/addon-a11y",
    // runs stories as Vitest component tests (Playwright Chromium); carries the
    // a11y test integration into the gate.
    "@storybook/addon-vitest",
    // light/dark via the .dark class (matches app.css)
    "@storybook/addon-themes",
  ],
  framework: { name: "@storybook/react-vite", options: {} },
};

export default config;
