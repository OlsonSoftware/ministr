/**
 * Human-friendly formatters shared across surfaces.
 *
 * Kept deliberately small — anything that could be a one-line
 * `Intl.NumberFormat` / `toLocaleString` call doesn't get wrapped here.
 * Each helper either encodes a project-specific copy convention (e.g.
 * "~5s left" vs. "in 5 seconds") or bridges Unix-second timestamps to
 * relative-time strings the design system expects.
 */

/**
 * Compact token-count formatter shared across overview / session /
 * activity-feed / turn-block displays. Returns "1.2M" / "3.4K" /
 * "850" depending on magnitude.
 */
export function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toString();
}

/** Format a seconds-remaining count as a compact "~Xs left" string. */
export function formatEta(secs: number): string {
  if (secs < 60) return `~${Math.max(1, Math.round(secs))}s left`;
  if (secs < 3600) return `~${Math.round(secs / 60)} min left`;
  return `~${(secs / 3600).toFixed(1)} h left`;
}

/**
 * Bare ETA without the "left" suffix — useful inside a fixed-width row
 * where the trailing word would jitter (onboarding step 2 uses this).
 */
export function formatEtaBare(secs: number): string {
  if (secs < 60) return `${Math.max(1, Math.round(secs))}s`;
  if (secs < 3600) return `${Math.round(secs / 60)} min`;
  return `${(secs / 3600).toFixed(1)} h`;
}

/**
 * Render a Unix-seconds timestamp as a relative-time phrase ("just now"
 * / "5 min ago" / etc.). Falls back to a localized date string for
 * anything older than a week.
 */
export function formatRelativeTime(unixSeconds: number): string {
  const now = Math.floor(Date.now() / 1000);
  const diff = now - unixSeconds;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)} min ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} h ago`;
  if (diff < 604800) return `${Math.floor(diff / 86400)} d ago`;
  return new Date(unixSeconds * 1000).toLocaleDateString();
}
