/**
 * relTime — a compact relative-time label from a Unix timestamp in SECONDS
 * (the daemon's CorpusInfo.last_indexed format). Calm + scannable: "just
 * now" / "3m ago" / "2h ago" / "yesterday" / "4d ago" / "3w ago" / a date.
 * Pure (nowMs injectable) so it's deterministically unit-testable.
 */
export function relTime(unixSeconds: number, nowMs: number = Date.now()): string {
  const deltaMs = nowMs - unixSeconds * 1000;
  if (!Number.isFinite(deltaMs)) return "";
  const sec = Math.floor(deltaMs / 1000);
  if (sec < 0) return "just now"; // clock skew / future stamp — don't lie
  if (sec < 45) return "just now";

  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;

  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;

  const day = Math.floor(hr / 24);
  if (day === 1) return "yesterday";
  if (day < 7) return `${day}d ago`;
  if (day < 28) return `${Math.floor(day / 7)}w ago`;

  // Older than ~a month: a short absolute date reads clearer than "5w ago".
  return new Date(unixSeconds * 1000).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
  });
}
