/**
 * Compact "Ns ago" / "Nm ago" / "Nh ago" formatter for activity and
 * coherence feeds. Both inputs are millisecond epochs.
 */
export function relative(nowMs: number, tsMs: number): string {
  const delta = Math.max(0, nowMs - tsMs);
  const secs = Math.floor(delta / 1000);
  if (secs < 1) return "now";
  if (secs < 60) return `${secs}s ago`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  return `${hrs}h ago`;
}
