import type { IngestionProgressInfo } from "./ipc";

/**
 * Scripted progress sequences for Storybook (gui-progress-data-hook): each
 * script is a stateful closure that returns the NEXT snapshot every time the
 * mocked `ingestion_progress` command is polled, so stories exercise the
 * derivation math against realistic motion — start, phase change, stall,
 * completion — without a daemon. Downstream instrument chunks develop
 * against these same scripts.
 */

const FILES = [
  "src/lib/retry.rs",
  "src/lib/quorum.rs",
  "src/daemon/registry.rs",
  "src/daemon/indexer.rs",
  "docs/architecture.md",
  "src/index/hnsw.rs",
];

function base(over: Partial<IngestionProgressInfo>): IngestionProgressInfo {
  return {
    corpus_id: "corpus-demo",
    status: 1,
    phase: "embedding",
    files_total: 240,
    files_done: 240,
    sections_done: 980,
    embeddings_total: 4200,
    embeddings_done: 0,
    current_file: FILES[0],
    ...over,
  };
}

/** A full realistic run: discover → parse → embed → finalize → complete. */
export function fullRunScript(): () => IngestionProgressInfo[] {
  let tick = 0;
  return () => {
    tick += 1;
    if (tick <= 2) {
      return [
        base({
          phase: "discovering",
          files_total: 0,
          files_done: 0,
          embeddings_done: 0,
        }),
      ];
    }
    if (tick <= 8) {
      const done = Math.min(240, (tick - 2) * 40);
      return [
        base({
          phase: "parsing",
          files_done: done,
          embeddings_done: 0,
          current_file: FILES[tick % FILES.length],
        }),
      ];
    }
    if (tick <= 28) {
      const done = Math.min(4200, (tick - 8) * 210);
      return [
        base({
          embeddings_done: done,
          current_file: FILES[tick % FILES.length],
        }),
      ];
    }
    return [
      base({ status: 2, phase: "idle", embeddings_done: 4200, current_file: "" }),
    ];
  };
}

/** Steady mid-run embedding at a constant rate — the ETA's happy path. */
export function steadyEmbedScript(): () => IngestionProgressInfo[] {
  let done = 1200;
  return () => {
    done = Math.min(4200, done + 150);
    return [base({ embeddings_done: done, current_file: FILES[done % FILES.length] })];
  };
}

/** Progress that freezes mid-run — the ETA must hide, never freeze. */
export function stallScript(): () => IngestionProgressInfo[] {
  let tick = 0;
  return () => {
    tick += 1;
    const done = tick <= 4 ? tick * 200 : 800; // moves, then stops
    return [base({ embeddings_done: done })];
  };
}

/** A finished run plus an idle second corpus. */
export function completeScript(): () => IngestionProgressInfo[] {
  return () => [
    base({ status: 2, phase: "idle", embeddings_done: 4200, current_file: "" }),
    base({
      corpus_id: "corpus-idle",
      status: 0,
      phase: "idle",
      files_total: 0,
      files_done: 0,
      embeddings_total: 0,
      embeddings_done: 0,
      current_file: "",
    }),
  ];
}
