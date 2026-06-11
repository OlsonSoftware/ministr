import { describe, expect, it } from "vitest";
import { derivePresence } from "./presence";
import type { ActivityEvent } from "./ipc";

const ev = (ts: number, corpus = "c1"): ActivityEvent => ({
  timestamp_ms: ts,
  tool: "ministr_read",
  corpus_id: corpus,
  summary: "src/auth.ts",
  cache_hit: false,
});

describe("presence is real (invariant 2)", () => {
  it("no events → no presence", () => {
    expect(derivePresence([], "c1", 1_000_000)).toBeNull();
  });

  it("another corpus's events never leak", () => {
    expect(derivePresence([ev(999_000, "other")], "c1", 1_000_000)).toBeNull();
  });

  it("fresh event → live with the event-true sentence", () => {
    const p = derivePresence([ev(995_000)], "c1", 1_000_000);
    expect(p).toEqual({ kind: "live", sentence: "your AI read auth.ts" });
  });

  it("older event → dim recent line; ancient → nothing", () => {
    const recent = derivePresence([ev(1_000_000 - 120_000)], "c1", 1_000_000);
    expect(recent?.kind).toBe("recent");
    expect(recent?.sentence).toBe(
      "your AI last looked at this project 2 minutes ago",
    );
    expect(derivePresence([ev(0)], "c1", 100 * 60_000)).toBeNull();
  });
});
