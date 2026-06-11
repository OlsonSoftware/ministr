import { describe, expect, it } from "vitest";
import { activitySentence, aggregate, buildFeed, outcomeSentence } from "./receipts";
import type { ActivityEvent, OutcomeEventInfo } from "./ipc";

const act = (tool: string, summary: string, ts = 1, cache = false): ActivityEvent => ({
  timestamp_ms: ts,
  tool,
  corpus_id: "c",
  summary,
  cache_hit: cache,
});

const outcome = (over: Partial<OutcomeEventInfo>): OutcomeEventInfo => ({
  session_id: "s",
  path: "/r/src/auth.ts",
  read_rank: 1,
  first_touch: true,
  reads_before: 0,
  edited_at_ms: 10,
  ...over,
});

describe("receipts restate events 1:1", () => {
  it("survey and read sentences", () => {
    expect(activitySentence(act("ministr_survey", "login button"))).toBe(
      "your AI searched “login button”",
    );
    expect(activitySentence(act("ministr_read", "/r/src/auth.ts#root"))).toBe(
      "your AI read auth.ts#root",
    );
  });

  it("unknown tools restate honestly, never invent", () => {
    expect(activitySentence(act("ministr_run", "cargo test"))).toBe(
      "your AI used run (cargo test)",
    );
  });

  it("outcome wording is the join fact, never 'read before edit'", () => {
    const first = outcomeSentence(outcome({}));
    expect(first.kind).toBe("win");
    expect(first.sentence).toBe("you changed auth.ts — the first file your AI read");

    const later = outcomeSentence(outcome({ first_touch: false, read_rank: 4 }));
    expect(later.kind).toBe("headsup");
    expect(later.sentence).toBe(
      "you changed auth.ts — your AI had read it (file #4 it looked at)",
    );
    expect(later.sentence).not.toMatch(/before/);
  });

  it("feed merges newest first", () => {
    const feed = buildFeed(
      [act("ministr_survey", "q", 5)],
      [outcome({ edited_at_ms: 9 }), outcome({ edited_at_ms: 1 })],
    );
    expect(feed.map((l) => l.ts)).toEqual([9, 5, 1]);
  });

  it("aggregate is counts only", () => {
    const text = aggregate(
      [act("ministr_survey", "q", 1, true), act("ministr_read", "f", 2)],
      { events: [outcome({})], stats: [] },
    );
    expect(text).toBe(
      "1 search · 1 read · 1 answered from memory · 1 file your AI read got edited (1 on its first read)",
    );
    expect(text).not.toMatch(/saved|faster|vs/);
  });
});
