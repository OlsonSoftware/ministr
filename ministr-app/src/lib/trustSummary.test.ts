import { describe, expect, it } from "vitest";
import { buildTree, leafNote, summarize, summarizeCounts } from "./trustSummary";
import type { FreshnessResponse } from "./ipc";

const fresh = (states: [string, "current" | "stale" | "new" | "missing"][]): FreshnessResponse => ({
  files: states.map(([path, state]) => ({ path, state })),
  indexing: false,
});

describe("summarize", () => {
  it("all current reads up to date", () => {
    const s = summarize("my-app", fresh([["a.ts", "current"]]));
    expect(s.state).toBe("ok");
    expect(s.headline).toMatch(/up to date/);
  });

  it("stale + new files count as behind, with honest counts", () => {
    const s = summarize(
      "my-app",
      fresh([
        ["a.ts", "stale"],
        ["b.ts", "new"],
        ["c.ts", "current"],
      ]),
    );
    expect(s.state).toBe("stale");
    expect(s.headline).toBe("Your AI is 2 files behind");
  });

  it("indexing wins as updating", () => {
    const s = summarize("my-app", { ...fresh([["a.ts", "stale"]]), indexing: true });
    expect(s.state).toBe("updating");
  });
});

describe("buildTree", () => {
  it("rolls worst state up through directories", () => {
    const tree = buildTree([
      { path: "src/ok.ts", state: "current" },
      { path: "src/lib/bad.ts", state: "stale" },
      { path: "docs/readme.md", state: "current" },
    ]);
    const src = tree.find((n) => n.name === "src");
    const docs = tree.find((n) => n.name === "docs");
    expect(src?.state).toBe("stale"); // worst-state-wins
    expect(docs?.state).toBe("ok");
    const lib = src?.children.find((n) => n.name === "lib");
    expect(lib?.state).toBe("stale");
  });

  it("sorts directories before files", () => {
    const tree = buildTree([
      { path: "zz.ts", state: "current" },
      { path: "aa/x.ts", state: "current" },
    ]);
    expect(tree[0].name).toBe("aa");
    expect(tree[1].name).toBe("zz.ts");
  });
});

describe("summarizeCounts (Home's cheap path)", () => {
  it("matches the full-path math for the same counts", () => {
    const s = summarizeCounts("app", { stale: 1, new: 1, indexing: false });
    expect(s.state).toBe("stale");
    expect(s.headline).toBe("Your AI is 2 files behind");
    expect(
      summarizeCounts("app", { stale: 0, new: 0, indexing: false }).state,
    ).toBe("ok");
    expect(
      summarizeCounts("app", { stale: 0, new: 0, indexing: true }).state,
    ).toBe("updating");
  });
});

describe("per-file updating during reindex (consistency-pass)", () => {
  const files = [
    { path: "src/old.ts", state: "stale" as const },
    { path: "src/brand-new.ts", state: "new" as const },
    { path: "src/gone.ts", state: "missing" as const },
    { path: "src/fine.ts", state: "current" as const },
  ];

  it("idle: behind files are stale, current is ok", () => {
    const tree = buildTree(files)[0];
    const states = Object.fromEntries(
      tree.children.map((c) => [c.name, c.state]),
    );
    expect(states["old.ts"]).toBe("stale");
    expect(states["fine.ts"]).toBe("ok");
  });

  it("indexing: stale/new flip to updating, missing stays behind, current stays ok", () => {
    const tree = buildTree(files, true)[0];
    const states = Object.fromEntries(
      tree.children.map((c) => [c.name, c.state]),
    );
    expect(states["old.ts"]).toBe("updating");
    expect(states["brand-new.ts"]).toBe("updating");
    expect(states["gone.ts"]).toBe("stale");
    expect(states["fine.ts"]).toBe("ok");
  });

  it("leafNote follows the live state", () => {
    expect(leafNote("stale", true)).toBe("being brought up to date right now");
    expect(leafNote("stale", false)).toBe("your AI sees an older version");
  });
});
