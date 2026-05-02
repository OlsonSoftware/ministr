// Shared helper for all sweep + audit scripts.
//
// Resolves `ministr-app/src` relative to this file so the scripts work
// from any clone location (CI containers, ~/work/ministr on Linux,
// D:/Code/ministr on Windows, etc.) instead of hard-coding the original
// author's machine path.
//
// Usage:
//   import { srcRoot } from "./_src-root.mjs";
//   const root = srcRoot;            // absolute path to ministr-app/src
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));

/** Absolute path to `ministr-app/src` regardless of where the repo lives. */
export const srcRoot = path.resolve(here, "..", "src");
