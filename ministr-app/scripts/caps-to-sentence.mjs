// Sweep .tsx files: find JSX text nodes that are all-caps prose / button
// labels and convert to sentence case. Whitelist preserves page titles,
// short labels, and the typed-confirm tokens.
import fs from "node:fs";
import path from "node:path";

const root = "D:/Code/ministr/ministr-app/src";

// Strings we MUST keep uppercase (page titles, section titles, short labels,
// magic-word tokens, kind tags). Everything else gets converted.
const KEEP_UPPER = new Set([
  // Page titles / role A
  "SEARCH",
  "SYMBOLS",
  "BRIDGE",
  "PROJECTS",
  "STRUCTURE",
  "SESSIONS",
  "LOGS",
  "SETTINGS",
  "READY",
  "DETAIL",
  // Wordmark + tagline
  "MINISTR",
  "CODE INTELLIGENCE",
  // Section header role B
  "OVERVIEW",
  "SIGNATURE",
  "DOCS",
  "SOURCE",
  "REFERENCES",
  "BRIDGES",
  "META",
  "EXPORT",
  "IMPORT",
  "STATS",
  "METADATA",
  "MAINTENANCE",
  "PREFERENCES",
  "ACTIONS",
  "ACTIVITY",
  "BUDGET",
  "CONNECTION",
  "CHANGES",
  "MATCHES",
  "BROWSE",
  "MENTIONS",
  "SAME FILE",
  "SAME FILE — SYMBOLS",
  "BRIDGES — EXPORT",
  "BRIDGES — IMPORT",
  "BRIDGES INVOLVING",
  "BRIDGE LINKS",
  "BRIDGE SURFACE",
  "RECENT CHANGES",
  "HOT FILES",
  "TOP FILES",
  "TOP FILES BY SECTIONS",
  "ACTIVE SESSIONS",
  "SURFACE",
  "TRAIL",
  "PROBES",
  "RECENT",
  "FACETS",
  "ALL",
  "INVERT",
  "PAUSE",
  "RESUME",
  "RUN",
  "GO",
  "NAV",
  "CORPUS",
  "LIVE",
  "HISTORY",
  "TAIL",
  "INFO",
  "WARN",
  "ERROR",
  "CRITICAL",
  "ELEVATED+",
  "COMPACT",
  "COMFORT",
  "OFF",
  "ON",
  "GROUP",
  "FLAT",
  "BY DIR",
  "BY EXT",
  "MIN SECTIONS",
  "LANG MIX",
  "LANG",
  "ANY LANG",
  "ANY",
  "KIND",
  "KIND DASHBOARD",
  "KIND BREAKDOWN",
  "BRIDGES OVERVIEW",
  "CONFIDENCE",
  "CONFIDENCE DISTRIBUTION",
  "MEM",
  "CORPORA",
  "TOKENS",
  "SAVED",
  "DEDUP",
  "PRESSURE",
  "POLL",
  "TURN",
  "FILES",
  "VECTORS",
  "CONF",
  "LANGS",
  "T",
  "RE",
  "Aa",
  "ESC",
  "MOVE",
  "RUN",
  "VERSION",
  "UPTIME",
  "MODEL",
  "DIM",
  "MEMORY",
  "DATA DIR",
  "LOG FILE",
  "DEFAULT TAB",
  "DENSITY",
  "AUTOSTART",
  "READ-ONLY",
  "THEME",
  "SYSTEM",
  "DARK",
  "LIGHT",
  // Kind tags (single short words)
  "FN",
  "STRUCT",
  "TRAIT",
  "ENUM",
  "IMPL",
  "TYPE",
  "MOD",
  "MODULE",
  "CONST",
  "PUB",
  // Reference kinds
  "CALLS",
  "IMPORTS",
  "USES",
  // Bridge kinds (already lowercase in source)
  // Coherence kinds
  "CREATED",
  "MODIFIED",
  "REMOVED",
  // Onboarding step keys + features row
  "WELCOME",
  "DETECT",
  "DONE",
  "SURVEY",
  "SYMBOLS",
  "REFERENCES",
  "BRIDGE",
  "TRY THIS",
  "CODE INTELLIGENCE FOR LLM AGENTS", // kept for hero tagline emphasis (single line)
  // Typed-confirm magic words
  "RESET",
  "CLEAR CACHE",
  "REMOVE",
  "RE-INDEX",
  "RE-INDEX",
  "REMOVE PROJECT",
  // ⌘K kbd-style hint
  "⌘K",
  // Misc tags
  "ADD",
  "SKIP",
  "SCAN",
  "BACK",
  "CANCEL",
  "RETRY",
  "OPEN",
  "CLOSE",
]);

function toSentenceCase(s) {
  // Preserve existing lowercase letters but lowercase everything else,
  // then uppercase the very first letter. Treat hyphens as words too.
  const lower = s.toLowerCase();
  return lower.charAt(0).toUpperCase() + lower.slice(1);
}

function isAllUpper(s) {
  // Pure A-Z + spaces + hyphens + digits + simple punct, has at least one A-Z.
  if (!/[A-Z]/.test(s)) return false;
  if (/[a-z]/.test(s)) return false;
  if (!/^[A-Z0-9\s\-·.,/&+!?'"’–—:;()]+$/.test(s)) return false;
  return true;
}

function shouldConvert(s) {
  const trimmed = s.trim();
  if (!trimmed) return false;
  if (!isAllUpper(trimmed)) return false;
  if (KEEP_UPPER.has(trimmed)) return false;
  // Keep tiny labels regardless (≤3 chars). Most are kbd / abbrevs.
  if (trimmed.length <= 3) return false;
  // Multi-word OR more than 8 chars triggers conversion.
  return trimmed.includes(" ") || trimmed.length > 8;
}

function walk(d) {
  return fs
    .readdirSync(d, { withFileTypes: true })
    .flatMap((e) =>
      e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)],
    );
}

const files = walk(root).filter((f) => f.endsWith(".tsx"));

let totalEdits = 0;
const editsByFile = {};
for (const f of files) {
  let src = fs.readFileSync(f, "utf8");
  let edits = 0;

  // Match JSX text nodes between > and < — content of an element. This is
  // approximate but works for our static labels. Skips JSX expression
  // braces and attributes.
  src = src.replace(/>([^<>{]+)</g, (m, inner) => {
    const t = inner;
    // The captured group includes possibly leading/trailing whitespace
    // including newlines + indentation. Operate only on the visible text.
    const stripped = t.replace(/\s+/g, " ").trim();
    if (!shouldConvert(stripped)) return m;
    edits++;
    // Convert just the text portion; preserve surrounding whitespace.
    const sentence = toSentenceCase(stripped);
    return ">" + t.replace(stripped, sentence) + "<";
  });

  if (edits) {
    fs.writeFileSync(f, src);
    totalEdits += edits;
    editsByFile[path.relative(root, f).replaceAll("\\", "/")] = edits;
  }
}

for (const [file, n] of Object.entries(editsByFile)) {
  console.log(file + ": +" + n);
}
console.log("total: " + totalEdits);
