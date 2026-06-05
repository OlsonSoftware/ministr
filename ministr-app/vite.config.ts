/// <reference types="vitest/config" />
import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { storybookTest } from "@storybook/addon-vitest/vitest-plugin";
import { playwright } from "@vitest/browser-playwright";

const dirname = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  plugins: [
    // React Compiler 1.0 (stable Oct 2025) — build-time auto-memoization across
    // the whole app, removing the manual memo/useCallback burden (only ~2/32
    // atoms were hand-memoized). Wired via @vitejs/plugin-react v4's Babel pass
    // (v6 switched Babel→oxc and would need a different setup). React 19 ships
    // the compiler runtime, so no react-compiler-runtime package is needed; the
    // compiler bails out (skips) any component that breaks the Rules of React
    // rather than miscompiling it.
    react({
      babel: {
        plugins: [["babel-plugin-react-compiler", { target: "19" }]],
      },
    }),
    tailwindcss(),
  ],
  // Mirror the tsconfig `@/* → ./src/*` path alias at the bundler layer so
  // runtime imports (app build + Storybook, which reuses this config) resolve
  // it too — not just tsc.
  resolve: {
    alias: { "@": path.resolve(dirname, "src") },
  },
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2021",
    minify: !process.env.TAURI_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_DEBUG,
  },
  test: {
    // Two projects under one `vitest run`:
    //  • unit       — happy-dom, the RTL/OOUX component tests (structure/behaviour).
    //  • storybook  — every story rendered in a real Playwright Chromium browser
    //                 via the Storybook Vitest addon, with axe (addon-a11y,
    //                 `a11y.test: "error"` in preview) failing the run on any WCAG
    //                 violation. A real browser is what makes color-contrast (and
    //                 thus the light+dark §9 floor) mechanically checkable.
    projects: [
      {
        extends: true,
        test: {
          name: "unit",
          // happy-dom is faster than jsdom for component tests.
          environment: "happy-dom",
          globals: true,
          setupFiles: ["./src/test/setup.ts"],
          // These tests assert structure/behaviour, not computed styles — skip
          // CSS processing so Tailwind/@import don't slow or break the run.
          css: false,
          include: ["src/**/*.{test,spec}.{ts,tsx}"],
        },
      },
      // Every story rendered as a component test in Playwright Chromium with
      // axe (a11y.test "error"). Run TWICE — once per theme — so the §9 WCAG
      // floor is enforced in BOTH light and dark (only the setup file differs:
      // the dark one adds the `.dark` class). Both force animations to their
      // final frame so axe never snapshots text mid-opacity-fade.
      storybookProject("storybook", ".storybook/test-setup.ts"),
      storybookProject("storybook-dark", ".storybook/test-setup-dark.ts"),
    ],
  },
});

function storybookProject(name: string, setupFile: string) {
  return {
    extends: true as const,
    plugins: [storybookTest({ configDir: path.join(dirname, ".storybook") })],
    test: {
      name,
      browser: {
        enabled: true,
        provider: playwright({}),
        headless: true,
        instances: [
          { browser: "chromium", context: { reducedMotion: "reduce" as const } },
        ],
      },
      setupFiles: [setupFile],
    },
  };
}
