import { describe, expect, it } from "vitest";
import type { ActivityEvent } from "./types";
import { summarizeCodeTouched } from "./session-activity-summary";

const ev = (over: Partial<ActivityEvent>): ActivityEvent => ({
  timestamp_ms: 0,
  tool: "ministr_read",
  corpus_id: "c",
  summary: "",
  cache_hit: false,
  duration_ms: 0,
  ...over,
});

describe("summarizeCodeTouched symbolRefs", () => {
  it("pairs each touched symbol with the file it was seen in", () => {
    const { symbolRefs } = summarizeCodeTouched([
      ev({
        tool: "ministr_definition",
        summary: "verify_token — /Users/a/Code/ministr/./src/auth/middleware.rs",
      }),
      ev({
        tool: "ministr_references",
        summary:
          "create_session — /Users/a/Code/ministr/./src/session/registry.rs (4)",
      }),
    ]);
    const byName = Object.fromEntries(symbolRefs.map((s) => [s.name, s.file]));
    expect(byName.verify_token).toBe("src/auth/middleware.rs");
    expect(byName.create_session).toBe("src/session/registry.rs");
  });

  it("upgrades a null file to a real one when a later event carries it", () => {
    const { symbolRefs } = summarizeCodeTouched([
      // A definition summary without the ` — file` form → symbol, no file.
      ev({ tool: "ministr_definition", summary: "Widget" }),
      ev({
        tool: "ministr_definition",
        summary: "Widget — /Users/a/Code/app/./src/ui/widget.ts",
      }),
    ]);
    const widget = symbolRefs.find((s) => s.name === "Widget");
    expect(widget?.file).toBe("src/ui/widget.ts");
  });

  it("keeps symbolRefs in sync with the distinct symbols list", () => {
    const summary = summarizeCodeTouched([
      ev({ tool: "ministr_definition", summary: "a — /r/./x.rs" }),
      ev({ tool: "ministr_definition", summary: "a — /r/./x.rs" }),
      ev({ tool: "ministr_references", summary: "b — /r/./y.rs (1)" }),
    ]);
    expect(summary.symbolRefs.map((s) => s.name)).toEqual(summary.symbols);
    expect(summary.symbolRefs).toHaveLength(2);
  });
});
