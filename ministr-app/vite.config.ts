/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

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
    // happy-dom is the 2026 default — faster than jsdom for component tests.
    environment: "happy-dom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    // Component tests assert structure/behaviour, not computed styles — skip
    // CSS processing so Tailwind/@import don't slow or break the run.
    css: false,
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
  },
});
