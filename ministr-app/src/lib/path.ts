/**
 * File-path display helpers.
 *
 * Centralizes the "make this absolute path readable in a corpus-scoped
 * UI" decision so list views (HotFilesTile, AskView sources, Bridge
 * table, etc.) don't each invent their own `slice(-2)` heuristic. The
 * rule: when a corpus is selected and a path lives inside it, drop the
 * corpus's longest-common-ancestor prefix and show the rest. Otherwise
 * fall back to the basename (or last two segments).
 */

import type { CorpusInfo } from "./types";
import { corpusRoot } from "./corpus";

/** Last segment of a Unix or Windows path. */
export function basename(path: string): string {
  const parts = path.replace(/[\\/]+$/, "").split(/[\\/]/);
  return parts[parts.length - 1] ?? path;
}

/** Drop the last segment, return the parent directory path. */
export function dirname(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const idx = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return idx > 0 ? trimmed.slice(0, idx) : "";
}

/**
 * Render `path` relative to the active corpus's root.
 *
 * - If `corpus` is null/undefined → returns `basename(path)`.
 * - If the corpus has no usable root → returns the last 2 segments.
 * - If `path` lives inside the corpus root → returns the suffix with
 *   `/` separators, no leading slash.
 * - If `path` doesn't start with the corpus root (cross-corpus link,
 *   absolute path from elsewhere) → returns the last 2 segments.
 *
 * Always normalizes Windows backslashes to forward slashes for
 * consistent display.
 */
export function corpusRelative(
  path: string,
  corpus: CorpusInfo | null | undefined,
): string {
  const norm = path.replace(/\\/g, "/");
  if (!corpus) return basename(norm);

  const root = corpusRoot(corpus.paths).replace(/\\/g, "/");
  if (!root) return lastTwoSegments(norm);

  // Compare case-insensitively on Windows-style paths so D:/foo vs.
  // d:/foo round-trip cleanly. Strict equality below would surface
  // the irrelevant case mismatch as "outside the corpus".
  const normLower = norm.toLowerCase();
  const rootLower = root.toLowerCase();
  const rootSlash = rootLower.endsWith("/") ? rootLower : `${rootLower}/`;
  if (normLower.startsWith(rootSlash)) {
    return norm.slice(rootSlash.length);
  }
  if (normLower === rootLower) {
    return basename(norm);
  }

  return lastTwoSegments(norm);
}

/** "Last two segments" fallback when we can't relativize cleanly. */
function lastTwoSegments(path: string): string {
  const parts = path.split("/").filter(Boolean);
  if (parts.length <= 2) return path.replace(/^[/\\]+/, "");
  return parts.slice(-2).join("/");
}
