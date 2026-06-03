/**
 * Shared types and helpers for the Ask surface.
 *
 * Lives next to the components instead of in `lib/` because every consumer
 * is inside `surfaces/ask/` and the shapes track the Tauri command's wire
 * format (`AskPhase`, `RecentEntry`) one-to-one.
 */

import { invoke } from "@tauri-apps/api/core";
import type { SymbolDefinitionDetail } from "../../../lib/types";

// ─────────────────────────────────────────────────────────────────────────────
// Wire types — match ministr-app/src-tauri/src/commands.rs::AskPhase exactly.

export type AskPhase =
  | { kind: "cache_hit"; source_ids: string[] }
  | {
      kind: "analyzed";
      sub_questions: string[];
      hyde_preview: string;
      symbol_hints: string[];
      bridge_relevant: boolean;
    }
  | {
      kind: "retrieved_candidates";
      by_strategy: Record<string, number>;
      merged_ids: string[];
    }
  | { kind: "reranked"; source_ids: string[] }
  | { kind: "retrieved"; source_ids: string[] }
  | { kind: "verified"; unsupported_claims: string[] }
  | {
      kind: "done";
      answer: string;
      source_ids: string[];
      cached: boolean;
      model: string;
      elapsed_ms: number;
    }
  | { kind: "error"; message: string };

/** Internal phase tracker — the seven raw `AskPhase` kinds collapse into
 *  these eight UI states. Plain-English labels live in {@link statusLabel}. */
export type AskPhaseName =
  | "idle"
  | "analyzing"
  | "retrieving"
  | "reranking"
  | "synthesizing"
  | "verifying"
  | "done"
  | "error";

export interface InferenceHealth {
  available: boolean;
  reason: string;
  binary_path: string | null;
}

export interface SectionDetailOut {
  section_id: string;
  heading_path: string[];
  text: string;
  summary: string | null;
  claims_available: number;
}

export interface RecentEntry {
  query: string;
  answer: string;
  source_ids: string[];
  cached: boolean;
  model: string;
  elapsed_ms: number;
  ts: number;
}

// ─────────────────────────────────────────────────────────────────────────────
// Plain-English phase mapping — collapses 5+ internal pipeline phases into
// the three states a human cares about. Per the jargon glossary:
//   analyzing | retrieving | reranking → "Thinking…"
//   synthesizing                       → "Writing answer…"
//   verifying                          → "Checking sources…"

export function statusLabel(phase: AskPhaseName): string | null {
  switch (phase) {
    case "analyzing":
    case "retrieving":
    case "reranking":
      return "Thinking…";
    case "synthesizing":
      return "Writing answer…";
    case "verifying":
      return "Checking sources…";
    default:
      return null;
  }
}

export function isLoadingPhase(phase: AskPhaseName): boolean {
  return (
    phase === "analyzing" ||
    phase === "retrieving" ||
    phase === "reranking" ||
    phase === "synthesizing" ||
    phase === "verifying"
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Recent answers — per-corpus localStorage cache. Cache key matches the
// daemon's `blake3(query)` so a second submit after a pin/unpin round-trips
// instantly via the daemon's own answer cache.

const RECENT_STORAGE_KEY = "ministr-ask-recent-v1";
export const RECENT_LIMIT = 10;

export function loadRecent(corpusId: string): RecentEntry[] {
  try {
    const raw = localStorage.getItem(RECENT_STORAGE_KEY);
    if (!raw) return [];
    const all = JSON.parse(raw) as Record<string, RecentEntry[]>;
    const list = all[corpusId];
    return Array.isArray(list) ? list.slice(0, RECENT_LIMIT) : [];
  } catch {
    return [];
  }
}

export function saveRecent(corpusId: string, entries: RecentEntry[]) {
  try {
    const raw = localStorage.getItem(RECENT_STORAGE_KEY);
    const all = (raw ? JSON.parse(raw) : {}) as Record<string, RecentEntry[]>;
    all[corpusId] = entries.slice(0, RECENT_LIMIT);
    localStorage.setItem(RECENT_STORAGE_KEY, JSON.stringify(all));
  } catch {
    /* localStorage unavailable — non-fatal */
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Pinned answers — a separate per-corpus localStorage namespace, replacing
// the multi-tab Investigation system. Pinning is a one-click "save this
// answer" action surfaced on the answer card.

const PINNED_STORAGE_KEY = "ministr-ask-pinned-v1";
export const PINNED_LIMIT = 20;

export function loadPinned(corpusId: string): RecentEntry[] {
  try {
    const raw = localStorage.getItem(PINNED_STORAGE_KEY);
    if (!raw) return [];
    const all = JSON.parse(raw) as Record<string, RecentEntry[]>;
    const list = all[corpusId];
    return Array.isArray(list) ? list.slice(0, PINNED_LIMIT) : [];
  } catch {
    return [];
  }
}

export function savePinned(corpusId: string, entries: RecentEntry[]) {
  try {
    const raw = localStorage.getItem(PINNED_STORAGE_KEY);
    const all = (raw ? JSON.parse(raw) : {}) as Record<string, RecentEntry[]>;
    all[corpusId] = entries.slice(0, PINNED_LIMIT);
    localStorage.setItem(PINNED_STORAGE_KEY, JSON.stringify(all));
  } catch {
    /* localStorage unavailable — non-fatal */
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Citation parsing.

/** Numeric citation indices (1-based) referenced in `[N]` / `[N, M]` form. */
export function citedIndices(answer: string): Set<number> {
  const set = new Set<number>();
  const re = /\[(\d+(?:\s*,\s*\d+)*)\]/g;
  let match;
  while ((match = re.exec(answer)) !== null) {
    for (const piece of match[1].split(",")) {
      const n = parseInt(piece.trim(), 10);
      if (Number.isFinite(n) && n > 0) set.add(n);
    }
  }
  return set;
}

/** Best-effort: extract a file path from a content_id like
 *  `d:/code/foo/bar.rs#root:c0` or `sym-d:/code/foo/bar.rs::mod::Sym`. */
export function filePathFromContentId(id: string): string {
  const noPrefix = id.replace(/^sym-/, "");
  const hashIdx = noPrefix.indexOf("#");
  const colonIdx = noPrefix.indexOf("::");
  let candidate: string;
  if (hashIdx >= 0) candidate = noPrefix.slice(0, hashIdx);
  else if (colonIdx >= 0) candidate = noPrefix.slice(0, colonIdx);
  else candidate = noPrefix;
  return candidate;
}

// ─────────────────────────────────────────────────────────────────────────────
// Source preview cache — flipping between loading and result states should
// not refetch. Lives at module scope so it survives component remounts when
// the user toggles tabs.

const sourcePreviewCache = new Map<
  string,
  { excerpt: string | null; headingPath: string[] | null }
>();

export async function fetchSourcePreview(
  corpusId: string,
  contentId: string,
): Promise<{ excerpt: string | null; headingPath: string[] | null }> {
  const cacheKey = `${corpusId}::${contentId}`;
  const cached = sourcePreviewCache.get(cacheKey);
  if (cached) return cached;

  const isSymbol = contentId.startsWith("sym-");
  let result: { excerpt: string | null; headingPath: string[] | null } = {
    excerpt: null,
    headingPath: null,
  };
  try {
    if (isSymbol) {
      const def = await invoke<SymbolDefinitionDetail>("symbol_definition", {
        corpusId,
        symbolId: contentId,
      });
      result = {
        excerpt: shortExcerpt(def.source_context || def.signature || ""),
        headingPath:
          def.heading_path && def.heading_path.length > 0
            ? def.heading_path
            : [`${def.kind} ${def.name}`],
      };
    } else {
      const det = await invoke<SectionDetailOut>("read_section", {
        corpusId,
        sectionId: contentId,
      });
      result = {
        excerpt: shortExcerpt(det.text),
        headingPath: det.heading_path,
      };
    }
  } catch {
    /* leave result empty on failure */
  }
  sourcePreviewCache.set(cacheKey, result);
  return result;
}

function shortExcerpt(text: string): string {
  // Preserve indentation + newlines so the excerpt can be syntax-highlighted
  // as real code (CodeExcerpt clamps the visible line count). Only trim
  // surrounding blank lines and cap the raw size so the preview cache stays
  // small.
  const trimmed = text.replace(/^\s*\n/, "").replace(/\s+$/, "");
  return trimmed.length > 600 ? trimmed.slice(0, 600) : trimmed;
}

// ─────────────────────────────────────────────────────────────────────────────
// Formatting.

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}
