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

/** A touched symbol paired with the file it was seen in, so the GUI can
 *  resolve it (search_symbols by name+file) and deep-link into Explore. */
export interface SymbolRef {
  name: string;
  /** Repo-relative file the symbol was touched in, or null if the event
   *  didn't carry one (then resolution falls back to a name-only search). */
  file: string | null;
}

export interface CodeTouchedSummary {
  files: FileBucket[];
  /** Distinct symbol short names (last `::` segment) the agent looked at. */
  symbols: string[];
  /** Distinct touched symbols paired with their file — drives the
   *  click-to-Explore deep-links (aaa-explore-session-codetouched). */
  symbolRefs: SymbolRef[];
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
  // name → file: keep the first file we see, but upgrade a null to a real
  // file if a later event for the same name carries one.
  const symbolFiles = new Map<string, string | null>();
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
      const existing = symbolFiles.get(parsed.symbol);
      if (existing == null && parsed.file) {
        symbolFiles.set(parsed.symbol, parsed.file);
      } else if (!symbolFiles.has(parsed.symbol)) {
        symbolFiles.set(parsed.symbol, parsed.file);
      }
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

  const symbolRefs: SymbolRef[] = Array.from(symbols)
    .sort()
    .map((name) => ({ name, file: symbolFiles.get(name) ?? null }));

  return {
    files,
    symbols: Array.from(symbols).sort(),
    symbolRefs,
    bridgeInspections,
    refsChecked,
  };
}

/**
 * Decompose an activity event into a structured display:
 *   head  — the primary, non-path label (symbol name, query, anchor)
 *   file  — repo-relative file path, rendered as a dim second line
 *   badge — small right-aligned chip pinned outside the truncate column
 *           (counts, `top_k=N`, kind filter, etc.) so it never gets cut
 *           when the head overflows.
 *
 * Pre-relativized; callers never see absolute paths.
 */
export interface FormattedActivity {
  head: string;
  file: string | null;
  badge: string | null;
}

/** Pull a trailing `(count)` or `(top_k=N)`-style suffix out of a head
 *  so the count can render as a right-pinned chip instead of being
 *  truncated as part of the head string. */
function extractBadge(s: string): { head: string; badge: string | null } {
  const m = s.match(/^(.*?)\s*\(([^()]+)\)\s*$/);
  if (!m) return { head: s.trim(), badge: null };
  const head = m[1].trim();
  const badge = m[2].trim();
  return { head, badge };
}

export function formatActivityForDisplay(e: ActivityEvent): FormattedActivity {
  const t = tag(e.tool);
  const raw = relativizeSummary((e.summary ?? "").trim());

  if (!raw) {
    return { head: e.corpus_id, file: null, badge: null };
  }

  switch (t) {
    case "definition": {
      const { head, badge } = extractBadge(raw);
      const { name, file } = splitNameDashFile(head);
      return withPromotedFile({ head: name || "", file, badge });
    }
    case "references": {
      const { head, badge } = extractBadge(raw);
      const { name, file } = splitNameDashFile(head);
      return withPromotedFile({ head: name || "", file, badge });
    }
    case "extract": {
      const { head, badge } = extractBadge(raw);
      // Head may be `anchor — file · "query"` / `file · "query"` / `file`.
      const stripped = stripQueryClause(head);
      const queryMatch = head.match(/·\s+(".+")$/);
      const query = queryMatch ? queryMatch[1] : null;
      const { name, file } = splitNameDashFile(stripped);
      const parts: string[] = [];
      if (name && file) parts.push(name);
      if (query) parts.push(query);
      return withPromotedFile({
        head: parts.join(" · "),
        file: file ?? stripped,
        badge,
      });
    }
    case "read": {
      const hashIdx = raw.indexOf("#");
      if (hashIdx < 0) return { head: raw, file: null, badge: null };
      return {
        head: raw.slice(hashIdx + 1),
        file: raw.slice(0, hashIdx),
        badge: null,
      };
    }
    case "toc": {
      const { head, badge } = extractBadge(raw);
      if (head === "<root>") {
        return { head: "<root>", file: null, badge };
      }
      // `head` IS the document file path. Treat as a file-only event:
      // file in the head slot, count in the badge slot.
      return { head, file: null, badge };
    }
    case "related": {
      const { head, badge } = extractBadge(raw);
      // claim_id is shaped like `<file>#<anchor>:cN`
      const hashIdx = head.indexOf("#");
      if (hashIdx < 0) return { head, file: null, badge };
      const file = head.slice(0, hashIdx);
      const claim = head.slice(hashIdx + 1);
      return { head: claim, file, badge };
    }
    default: {
      // survey / symbols / bridge / compress / dropped — extract any
      // trailing `(...)` clause as a badge, no file.
      const { head, badge } = extractBadge(raw);
      return { head, file: null, badge };
    }
  }
}

/** Promote a file into the head when the head is empty, so file-only
 *  events (read, toc) render as a single normal-weight line instead of
 *  an empty top line above a small file line. */
function withPromotedFile(s: FormattedActivity): FormattedActivity {
  if (!s.head && s.file) return { head: s.file, file: null, badge: s.badge };
  return s;
}
