import { describe, expect, it } from "vitest";
import type { IngestionProgressInfo } from "./ipc";
import { ProgressTracker } from "./progress";

function snap(over: Partial<IngestionProgressInfo>): IngestionProgressInfo {
  return {
    corpus_id: "c1",
    status: 1,
    phase: "embedding",
    files_total: 100,
    files_done: 100,
    sections_done: 400,
    embeddings_total: 1000,
    embeddings_done: 0,
    current_file: "src/lib/example.rs",
    ...over,
  };
}

describe("ProgressTracker", () => {
  it("hides the ETA until the rate has stabilized, then projects it", () => {
    const t = new ProgressTracker();
    // 100 embeddings/sec, polled every second.
    let d = t.observe(snap({ embeddings_done: 0 }), 0);
    expect(d.etaSeconds).toBeNull(); // first sight — no rate yet
    d = t.observe(snap({ embeddings_done: 100 }), 1000);
    expect(d.etaSeconds).toBeNull(); // 1 forward sample
    d = t.observe(snap({ embeddings_done: 200 }), 2000);
    expect(d.etaSeconds).toBeNull(); // 2 forward samples
    d = t.observe(snap({ embeddings_done: 300 }), 3000);
    // 3 forward samples at a steady 100/s with 700 remaining → ~7s.
    expect(d.etaSeconds).not.toBeNull();
    expect(d.etaSeconds).toBeGreaterThanOrEqual(6);
    expect(d.etaSeconds).toBeLessThanOrEqual(8);
    expect(d.ratePerSec).toBeCloseTo(100, 0);
    expect(d.percent).toBeCloseTo(0.3, 5);
  });

  it("never reports a negative ETA, even when done overshoots total", () => {
    const t = new ProgressTracker();
    t.observe(snap({ embeddings_done: 0 }), 0);
    t.observe(snap({ embeddings_done: 400 }), 1000);
    t.observe(snap({ embeddings_done: 800 }), 2000);
    const d = t.observe(snap({ embeddings_done: 1200 }), 3000); // > total
    expect(d.etaSeconds).not.toBeNull();
    expect(d.etaSeconds).toBeGreaterThanOrEqual(0);
    expect(d.percent).toBe(1); // clamped
  });

  it("hides the ETA during a stall instead of freezing a countdown", () => {
    const t = new ProgressTracker();
    t.observe(snap({ embeddings_done: 0 }), 0);
    t.observe(snap({ embeddings_done: 100 }), 1000);
    t.observe(snap({ embeddings_done: 200 }), 2000);
    let d = t.observe(snap({ embeddings_done: 300 }), 3000);
    expect(d.etaSeconds).not.toBeNull();
    // No forward progress for >5s → stalled, ETA hidden.
    d = t.observe(snap({ embeddings_done: 300 }), 9000);
    expect(d.stalled).toBe(true);
    expect(d.etaSeconds).toBeNull();
    // Progress resumes → stall clears and the ETA can return.
    d = t.observe(snap({ embeddings_done: 400 }), 10_000);
    expect(d.stalled).toBe(false);
    expect(d.etaSeconds).not.toBeNull();
  });

  it("treats a counter reset as a new run", () => {
    const t = new ProgressTracker();
    t.observe(snap({ embeddings_done: 0 }), 0);
    t.observe(snap({ embeddings_done: 500 }), 1000);
    t.observe(snap({ embeddings_done: 900 }), 2000);
    // New run: counter went backwards. No negative rate, ETA hidden again.
    const d = t.observe(snap({ embeddings_done: 50 }), 3000);
    expect(d.etaSeconds).toBeNull();
    expect(d.ratePerSec).toBeNull();
  });

  it("resets the rate on a phase change (files/s never blends into embeddings/s)", () => {
    const t = new ProgressTracker();
    t.observe(snap({ phase: "parsing", files_done: 0, files_total: 100 }), 0);
    t.observe(snap({ phase: "parsing", files_done: 50, files_total: 100 }), 1000);
    t.observe(snap({ phase: "parsing", files_done: 100, files_total: 100 }), 2000);
    const d = t.observe(snap({ phase: "embedding", embeddings_done: 10 }), 3000);
    expect(d.phase).toBe("embedding");
    expect(d.ratePerSec).toBeNull(); // fresh unit, no samples yet
    expect(d.etaSeconds).toBeNull();
  });

  it("is identity-guarded: re-observing the same snapshot does not decay the rate", () => {
    const t = new ProgressTracker();
    t.observe(snap({ embeddings_done: 0 }), 0);
    t.observe(snap({ embeddings_done: 100 }), 1000);
    const s = snap({ embeddings_done: 200 });
    const first = t.observe(s, 2000);
    // StrictMode double-render: same object, slightly later clock.
    const second = t.observe(s, 2005);
    expect(second).toBe(first);
    expect(second.ratePerSec).toBeCloseTo(first.ratePerSec as number, 10);
  });

  it("reports percent from the phase-appropriate counters and completion as 1", () => {
    const t = new ProgressTracker();
    let d = t.observe(
      snap({ phase: "discovering", files_done: 0, files_total: 0 }),
      0,
    );
    expect(d.percent).toBeNull(); // total unknown — no fake percent
    d = t.observe(
      snap({ phase: "parsing", files_done: 25, files_total: 100 }),
      1000,
    );
    expect(d.percent).toBeCloseTo(0.25, 5);
    d = t.observe(snap({ status: 2, embeddings_done: 1000 }), 2000);
    expect(d.complete).toBe(true);
    expect(d.percent).toBe(1);
  });

  it("tracks corpora independently", () => {
    const t = new ProgressTracker();
    t.observe(snap({ corpus_id: "a", embeddings_done: 100 }), 0);
    const d = t.observe(snap({ corpus_id: "b", embeddings_done: 0 }), 0);
    expect(d.corpusId).toBe("b");
    expect(d.ratePerSec).toBeNull(); // b has no history despite a's progress
  });
});
