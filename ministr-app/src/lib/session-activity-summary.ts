/**
 * Pure aggregators that turn an `ActivityEvent[]` stream into the
 * "Code touched" view: which files / symbols / bridges the agent
 * worked with this session.
 *
 * Contract — the daemon writes `ActivityEvent.summary` strings in a
 * tolerant format (see `ministr-daemon/src/daemon.rs::with_summary`):
 *
 *   definition  →  `{name} — {file}`
 *   references  →  `{name} — {file} ({n})`
 *   extract     →  `{anchor} — {file}[ · "{q}"] ({n})`  OR  `{file}[ · "{q}"] ({n})`
 *   read        →  `{file}[#{anchor}]`  (URL-derived fallback)
 *   bridge      →  `["{q}"][ · kind=X][ · lang=Y] ({n})`
 *   survey/symbols/toc/related/compress/dropped: no file info — ignored
 *
 * Parsing is tolerant: an event whose summary doesn't match the
 * pattern simply doesn't contribute to file/symbol aggregations.
 * The activity timeline still renders it.
 */
import type { ActivityEvent } from "./types";

/** Per-file activity counts. */
export interface FileBucket {
  file: string;
  reads: number;
  defs: number;
  refs: number;
  extracts: number;
  total: number;
}

export interface CodeTouchedSummary {
  files: FileBucket[];
  /** Distinct symbol short names (last `::` segment) the agent looked at. */
  symbols: string[];
  /** Number of bridge-inspection events. */
  bridgeInspections: number;
  /** Sum of result counts on references events — how many call sites
   *  the agent has audited this session. */
  refsChecked: number;
}

/** Strip a trailing ` (N)` count from a summary string, returning the
 *  cleaned head and the count if present. */
function splitTrailingCount(s: string): { head: string; count: number | null } {
  const m = s.match(/^(.*)\s+\((\d+)\)\s*$/);
  if (!m) return { head: s.trim(), count: null };
  return { head: m[1].trim(), count: Number.parseInt(m[2], 10) };
}

/** Extract a `{name} — {file}` pair from a summary head, if present. */
function splitNameDashFile(head: string): { name: string; file: string | null } {
  const idx = head.indexOf(" — ");
  if (idx < 0) return { name: head, file: null };
  return { name: head.slice(0, idx), file: head.slice(idx + 3) };
}

/** Strip a trailing ` · "query"` clause from the head of an extract
 *  summary so the file portion is recoverable. */
function stripQueryClause(s: string): string {
  return s.replace(/\s+·\s+".+"$/, "").trim();
}

/** Tool name normalised to its tag form (`ministr_definition` → `definition`). */
function tag(tool: string): string {
  return tool.replace(/^ministr_/, "").toLowerCase();
}

/** Information parsed from a single event's `summary`. All fields are
 *  best-effort — `null` means the parser couldn't recover that piece. */
interface ParsedEvent {
  file: string | null;
  symbol: string | null;
  count: number | null;
  isBridge: boolean;
}

function parseEvent(e: ActivityEvent): ParsedEvent {
  const t = tag(e.tool);
  const raw = (e.summary ?? "").trim();
  if (!raw) {
    return { file: null, symbol: null, count: null, isBridge: t === "bridge" };
  }

  switch (t) {
    case "definition": {
      const { head } = splitTrailingCount(raw);
      const { name, file } = splitNameDashFile(head);
      return { file, symbol: name || null, count: null, isBridge: false };
    }
    case "references": {
      const { head, count } = splitTrailingCount(raw);
      const { name, file } = splitNameDashFile(head);
      return { file, symbol: name || null, count, isBridge: false };
    }
    case "extract": {
      const { head } = splitTrailingCount(raw);
      const stripped = stripQueryClause(head);
      const { file } = splitNameDashFile(stripped);
      // If there's no "name — file" form, the head itself is the file path.
      return {
        file: file ?? stripped,
        symbol: null,
        count: null,
        isBridge: false,
      };
    }
    case "read": {
      // Summary is `<section_id>` = `<file>[#anchor]` (middleware fallback).
      const hashIdx = raw.indexOf("#");
      const file = hashIdx >= 0 ? raw.slice(0, hashIdx) : raw;
      return { file, symbol: null, count: null, isBridge: false };
    }
    case "bridge": {
      return { file: null, symbol: null, count: null, isBridge: true };
    }
    default:
      return { file: null, symbol: null, count: null, isBridge: false };
  }
}

/** Aggregate per-file buckets, distinct symbols, bridge count, and
 *  total refs checked from a stream of events. */
export function summarizeCodeTouched(
  events: ActivityEvent[],
): CodeTouchedSummary {
  const buckets = new Map<string, FileBucket>();
  const symbols = new Set<string>();
  let bridgeInspections = 0;
  let refsChecked = 0;

  for (const e of events) {
    const t = tag(e.tool);
    const parsed = parseEvent(e);

    if (parsed.isBridge) {
      bridgeInspections += 1;
    }

    if (t === "references" && parsed.count) {
      refsChecked += parsed.count;
    }

    if (parsed.symbol) {
      symbols.add(parsed.symbol);
    }

    if (parsed.file) {
      const file = parsed.file;
      const b = buckets.get(file) ?? {
        file,
        reads: 0,
        defs: 0,
        refs: 0,
        extracts: 0,
        total: 0,
      };
      switch (t) {
        case "read":
          b.reads += 1;
          break;
        case "definition":
          b.defs += 1;
          break;
        case "references":
          b.refs += 1;
          break;
        case "extract":
          b.extracts += 1;
          break;
        default:
          break;
      }
      b.total += 1;
      buckets.set(file, b);
    }
  }

  // Sort files by descending event count, then alphabetically as a
  // deterministic tiebreaker.
  const files = Array.from(buckets.values()).sort((a, b) => {
    if (b.total !== a.total) return b.total - a.total;
    return a.file.localeCompare(b.file);
  });

  return {
    files,
    symbols: Array.from(symbols).sort(),
    bridgeInspections,
    refsChecked,
  };
}

/** Render a file path as a compact display label — drops a long
 *  leading path so the visible portion fits in one line. */
export function shortFilePath(file: string, max = 56): string {
  if (file.length <= max) return file;
  // Keep the last segment(s); replace the head with `…`.
  const keep = file.length - (max - 1);
  return `…${file.slice(keep)}`;
}
