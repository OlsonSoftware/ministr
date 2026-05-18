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

/**
 * Strip the corpus-root prefix from a path so what we display is always
 * repo-relative, never absolute. ministr stores paths as
 * `/Users/<…>/<repo>/./<rel/path>` — the `/./` marker is the boundary
 * between the absolute prefix and the repo-relative portion, so we just
 * cut everything up to (and including) it.
 *
 * For paths without the marker (e.g. virtual sources like `web://…`),
 * returns the input unchanged.
 */
export function relativizePath(path: string): string {
  const m = path.match(/^.*?\/\.\//);
  return m ? path.slice(m[0].length) : path;
}

/**
 * Apply `relativizePath`-style stripping to any absolute-looking path
 * substring anywhere in a free-form summary. Used by the activity-row
 * renderer so `{name} — /Users/<…>/<repo>/./<rel>` becomes
 * `{name} — <rel>` for display without changing the underlying event.
 *
 * The pattern matches one or more `/<segment>` runs ending in the
 * literal `/./` corpus-root marker — so it cannot eat plain prose
 * like `"a · b · c"`.
 */
const ABS_PATH_PREFIX_RE = /(?:\/[^\s/]+)+\/\.\//g;

export function relativizeSummary(s: string): string {
  return s.replace(ABS_PATH_PREFIX_RE, "");
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

  // Helper: relativize a file path before storing it as a bucket key, so
  // the dashboard always groups by repo-relative paths.
  const rel = (f: string | null): string | null =>
    f ? relativizePath(f) : null;

  switch (t) {
    case "definition": {
      const { head } = splitTrailingCount(raw);
      const { name, file } = splitNameDashFile(head);
      return {
        file: rel(file),
        symbol: name || null,
        count: null,
        isBridge: false,
      };
    }
    case "references": {
      const { head, count } = splitTrailingCount(raw);
      const { name, file } = splitNameDashFile(head);
      return {
        file: rel(file),
        symbol: name || null,
        count,
        isBridge: false,
      };
    }
    case "extract": {
      const { head } = splitTrailingCount(raw);
      const stripped = stripQueryClause(head);
      const { file } = splitNameDashFile(stripped);
      // If there's no "name — file" form, the head itself is the file path.
      return {
        file: rel(file ?? stripped),
        symbol: null,
        count: null,
        isBridge: false,
      };
    }
    case "read": {
      // Summary is `<section_id>` = `<file>[#anchor]` (middleware fallback).
      const hashIdx = raw.indexOf("#");
      const file = hashIdx >= 0 ? raw.slice(0, hashIdx) : raw;
      return { file: rel(file), symbol: null, count: null, isBridge: false };
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

/**
 * Decompose an activity event into a structured display:
 *   head — the primary, non-path label (symbol name, query, anchor, etc.)
 *   file — the repo-relative file path, if the event has one; rendered
 *          on a secondary line in the activity row.
 *
 * Both fields are pre-relativized — callers never need to worry about
 * absolute paths leaking into the UI.
 */
export interface FormattedActivity {
  head: string;
  file: string | null;
}

export function formatActivityForDisplay(e: ActivityEvent): FormattedActivity {
  const t = tag(e.tool);
  const raw = relativizeSummary((e.summary ?? "").trim());

  if (!raw) {
    return { head: e.corpus_id, file: null };
  }

  switch (t) {
    case "definition": {
      const { head } = splitTrailingCount(raw);
      const { name, file } = splitNameDashFile(head);
      return withPromotedFile({ head: name || "", file });
    }
    case "references": {
      const { head, count } = splitTrailingCount(raw);
      const { name, file } = splitNameDashFile(head);
      const label = count != null && name ? `${name} (${count})` : name;
      return withPromotedFile({ head: label || "", file });
    }
    case "extract": {
      const { head, count } = splitTrailingCount(raw);
      // Head may be `anchor — file · "query"` or `file · "query"` or `file`.
      const stripped = stripQueryClause(head);
      const queryMatch = head.match(/·\s+(".+")$/);
      const query = queryMatch ? queryMatch[1] : null;
      const { name, file } = splitNameDashFile(stripped);
      // Anchor goes in head if present; otherwise just the query.
      const parts: string[] = [];
      if (name && file) parts.push(name);
      if (query) parts.push(query);
      const joined = parts.join(" · ");
      const headLabel = count != null
        ? joined
          ? `${joined} (${count})`
          : `(${count})`
        : joined;
      return withPromotedFile({
        head: headLabel,
        file: file ?? stripped,
      });
    }
    case "read": {
      const hashIdx = raw.indexOf("#");
      if (hashIdx < 0) return { head: raw, file: null };
      return { head: raw.slice(hashIdx + 1), file: raw.slice(0, hashIdx) };
    }
    case "toc": {
      const { head, count } = splitTrailingCount(raw);
      if (head === "<root>") {
        return {
          head: count != null ? `<root> (${count})` : "<root>",
          file: null,
        };
      }
      // `head` IS the document file path. Treat like a file-only event
      // (read-style): file goes on the single label line with the count
      // appended, no second line — putting just `(N)` on line 1 with the
      // file beneath reads as orphan metadata.
      return {
        head: count != null ? `${head} (${count})` : head,
        file: null,
      };
    }
    case "related": {
      const { head, count } = splitTrailingCount(raw);
      // claim_id is shaped like `<file>#<anchor>:cN`
      const hashIdx = head.indexOf("#");
      if (hashIdx < 0) {
        return {
          head: count != null ? `${head} (${count})` : head,
          file: null,
        };
      }
      const file = head.slice(0, hashIdx);
      const claim = head.slice(hashIdx + 1);
      return {
        head: count != null ? `${claim} (${count})` : claim,
        file,
      };
    }
    default:
      // survey / symbols / bridge / compress / dropped — no file in summary.
      return { head: raw, file: null };
  }
}

/** Promote a file into the head when the head is empty, so file-only
 *  events (read, toc) render as a single normal-weight line instead of
 *  an empty top line above a small file line. */
function withPromotedFile(s: FormattedActivity): FormattedActivity {
  if (!s.head && s.file) return { head: s.file, file: null };
  return s;
}
