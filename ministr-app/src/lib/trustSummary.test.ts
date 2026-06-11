import { describe, expect, it } from "vitest";
import { buildTree, summarize } from "./trustSummary";
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
