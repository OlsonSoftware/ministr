import { describe, expect, it } from "vitest";
import { relTime } from "./relTime";

// A fixed "now" so the buckets are deterministic.
const NOW = Date.UTC(2026, 5, 15, 12, 0, 0); // 2026-06-15T12:00:00Z
const sAgo = (s: number) => Math.floor(NOW / 1000) - s;

describe("relTime", () => {
  it("collapses the last ~minute to 'just now'", () => {
    expect(relTime(sAgo(0), NOW)).toBe("just now");
    expect(relTime(sAgo(44), NOW)).toBe("just now");
  });

  it("counts minutes, then hours, then days", () => {
    expect(relTime(sAgo(60), NOW)).toBe("1m ago");
    expect(relTime(sAgo(59 * 60), NOW)).toBe("59m ago");
    expect(relTime(sAgo(2 * 3600), NOW)).toBe("2h ago");
    expect(relTime(sAgo(24 * 3600), NOW)).toBe("yesterday");
    expect(relTime(sAgo(4 * 86400), NOW)).toBe("4d ago");
    expect(relTime(sAgo(14 * 86400), NOW)).toBe("2w ago");
  });

  it("falls back to a short date past ~a month", () => {
    // 60 days before 2026-06-15 ≈ mid-April.
    expect(relTime(sAgo(60 * 86400), NOW)).toMatch(/^[A-Z][a-z]{2} \d{1,2}$/);
  });

  it("never lies about future / skewed stamps", () => {
    expect(relTime(sAgo(-100), NOW)).toBe("just now");
  });
});
