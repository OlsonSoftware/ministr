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
