/**
 * Pure path helpers for the Code surface's file tree.
 *
 * Indexed file paths come back from the daemon as full (often absolute) stored
 * paths — e.g. `/Users/alrik/Code/ministr/ministr-core/src/lib.rs`. Rendering
 * the tree straight from those roots it at the filesystem root, burying the
 * project under empty `Users/alrik/Code/...` levels. These helpers compute the
 * longest shared directory prefix so the tree can root at the highest level the
 * corpus' files actually share, while callers keep the full path as the key
 * they hand back to `read_file`.
 */

/** Split a path into non-empty `/`-separated segments. */
function segments(path: string): string[] {
  return path.split("/").filter(Boolean);
}

/**
 * The longest common *directory* prefix of `paths`, as a `/`-joined string
 * (no trailing slash; leading slash preserved when every path is absolute).
 *
 * - Only whole directory segments count (never a partial segment).
 * - The last segment of each path is treated as a file name and never part of
 *   the prefix, so a single file yields its containing directory.
 * - Returns `""` when there is no shared directory (mixed roots).
 */
export function commonDirPrefix(paths: string[]): string {
  if (paths.length === 0) return "";
  const absolute = paths.every((p) => p.startsWith("/"));
  // Directory segments only: drop each path's final (file-name) segment.
  const dirSegs = paths.map((p) => {
    const s = segments(p);
    return s.slice(0, Math.max(0, s.length - 1));
  });

  let prefix = dirSegs[0];
  for (const segs of dirSegs.slice(1)) {
    let i = 0;
    while (i < prefix.length && i < segs.length && prefix[i] === segs[i]) i++;
    prefix = prefix.slice(0, i);
    if (prefix.length === 0) break;
  }
  if (prefix.length === 0) return "";
  return (absolute ? "/" : "") + prefix.join("/");
}

/**
 * Strip `prefix` (a `commonDirPrefix` result) from the front of `path`,
 * returning the remainder relative to the prefix. If `prefix` is empty or not
 * actually a prefix of `path`, the path is returned unchanged (minus a leading
 * slash) so display never breaks.
 */
export function stripPrefix(path: string, prefix: string): string {
  if (!prefix) return path.replace(/^\/+/, "");
  if (path === prefix) return "";
  const withSlash = prefix.endsWith("/") ? prefix : `${prefix}/`;
  if (path.startsWith(withSlash)) return path.slice(withSlash.length);
  return path.replace(/^\/+/, "");
}
