// Mechanical Phase-2 sweep:
//
//   1. Page-title <h1|h2|h3> with `font-mono ... uppercase ... text-text`
//      → swap className to Plex Serif sentence-case display.
//      Adjacent ALL-CAPS string content gets sentence-cased.
//
//   2. Static-container `border-2 border-border bg-surface` (cards/panels)
//      → `border border-border-soft bg-surface` (hairline 1px).
//      Excluded: any line that also contains `onClick=`, `<button`, `<input`,
//      `<kbd`, `bg-accent` (active state), `border-l-4`, `border-l-[`,
//      or sits within a className for an interactive control. We detect
//      interactive intent loosely; risk is leaving hits for hand cleanup,
//      not over-editing.
//
//   3. Section header `font-mono text-[0.6875rem] font-bold uppercase
//      tracking-[0.05em] text-text` (legacy EntitySection) → keep mono caps
//      for ≤14ch labels (Role B), but loosen to `font-serif text-base
//      font-bold` when the surrounding label text is multi-word sentence
//      case. This pass is conservative: mono uppercase short labels stay.
//
//   4. `border-2 border-border` on bare `<input>` and `<kbd>` — keep, but
//      not part of this script. Hand-edit primitives separately.
//
// Idempotent. Reports per-file edit counts.
import fs from "node:fs";
import path from "node:path";

const root = "D:/Code/ministr/ministr-app/src";

function walk(d) {
  return fs
    .readdirSync(d, { withFileTypes: true })
    .flatMap((e) =>
      e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)],
    );
}

const files = walk(root).filter((f) => /\.tsx$/.test(f));

const PRESERVE_TOKENS = ["LLM", "API", "HTTP", "IPC", "FFI", "URL", "IDE", "CLI",
  "NAPI", "JSON", "XML", "SQL", "UI", "OS", "PYO3", "WASM", "TLS", "TS", "JS",
  "RS", "PY", "MCP", "ID", "AI"];

function toSentence(s) {
  const tokens = s.split(/(\s+)/);
  const lowered = tokens
    .map((tok) => {
      if (/^\s+$/.test(tok)) return tok;
      const m = tok.match(/^([A-Z0-9⌘·.,!?:;\-/&()'"’]+?)([.,!?:;\-)]*)$/);
      if (!m) return tok.toLowerCase();
      const word = m[1];
      const trail = m[2];
      if (PRESERVE_TOKENS.includes(word.toUpperCase()) && word === word.toUpperCase()) {
        return word + trail;
      }
      return word.toLowerCase() + trail;
    })
    .join("");
  const idx = lowered.search(/[a-zA-Z]/);
  if (idx < 0) return lowered;
  return lowered.slice(0, idx) + lowered[idx].toUpperCase() + lowered.slice(idx + 1);
}

const REPLACEMENTS = {};

for (const f of files) {
  const before = fs.readFileSync(f, "utf8");
  let src = before;
  const stats = {};

  // --- (1) Page-title h1/h2 with mono+uppercase+text-text → serif display.
  // Regex matches: <h2|h3 className="...font-mono text-{lg|base} font-bold uppercase tracking-[Xem] text-text...">
  // We rewrite className and (if hardcoded UPPERCASE text follows) sentence-case the body.
  src = src.replace(
    /<(h[1-3])\s+className="([^"]*?)\bfont-mono\b([^"]*?)\bfont-bold\b([^"]*?)\buppercase\b([^"]*?)\btracking-\[[^\]]*\]([^"]*?)\btext-text\b([^"]*)">/g,
    (m, tag, a, b, c, d, e, f2) => {
      stats["heading→serif"] = (stats["heading→serif"] ?? 0) + 1;
      // Preserve any layout classes (flex, items-center, gap-*, etc.)
      const layout = [a, b, c, d, e, f2]
        .join(" ")
        .replace(/\s+/g, " ")
        .trim();
      const cleaned = layout
        .split(/\s+/)
        .filter((c) => !/^font-/.test(c) && !/^tracking-/.test(c) && !/^text-(xs|sm|base|lg|xl|2xl|3xl|\[)/.test(c) && c !== "uppercase")
        .join(" ");
      return `<${tag} className="font-serif text-2xl font-normal text-text leading-tight ${cleaned}">`;
    },
  );

  // --- (2) Static container `border-2 border-border bg-surface` → hairline.
  // We act per-line and skip lines containing interactive markers.
  src = src
    .split("\n")
    .map((line) => {
      if (!/border-2 border-border\b/.test(line)) return line;
      // Don't touch lines that ALSO have these markers (interactive intent).
      const interactive =
        /onClick=/.test(line) ||
        /<button/.test(line) ||
        /<input/.test(line) ||
        /<kbd/.test(line) ||
        /\bbg-accent\b/.test(line) ||
        /\bborder-accent\b/.test(line) ||
        /\bhover:bg-accent\b/.test(line) ||
        /\bhover:border-/.test(line) ||
        /\bcursor-pointer\b/.test(line) ||
        /\b-ml-\[2px\]\b/.test(line) || // joined-segmented control
        /\bfocus:bg-accent\b/.test(line);
      if (interactive) return line;
      // Don't touch the EntityPanel drawer border (kept signature).
      if (/\bborder-l-2\b/.test(line)) return line;
      const next = line.replace(
        /border-2 border-border\b/g,
        "border border-border-soft",
      );
      if (next !== line) {
        stats["card→hairline"] = (stats["card→hairline"] ?? 0) + 1;
      }
      return next;
    })
    .join("\n");

  // --- (3) Sentence-case adjacent ALL-CAPS in headings we just touched.
  // Find <h1|h2|h3 className="font-serif ..."> ... </h1|h2|h3> and convert.
  src = src.replace(
    /<(h[1-3])\s+className="font-serif[^"]*">\s*([^<>{][^<]*?)\s*<\/\1>/g,
    (m, tag, body) => {
      const trimmed = body.trim();
      // Convert if all-caps OR has trailing tags but is multi-word caps
      if (/^[A-Z][A-Z0-9\s\-·.,!?:;'"’]+$/.test(trimmed) && trimmed.length > 3) {
        stats["heading-text→sentence"] = (stats["heading-text→sentence"] ?? 0) + 1;
        return m.replace(trimmed, toSentence(trimmed));
      }
      return m;
    },
  );

  if (Object.keys(stats).length) {
    fs.writeFileSync(f, src);
    REPLACEMENTS[path.relative(root, f).replaceAll("\\", "/")] = stats;
  }
}

const totals = {};
for (const [file, stats] of Object.entries(REPLACEMENTS)) {
  console.log(file);
  for (const [k, v] of Object.entries(stats)) {
    totals[k] = (totals[k] ?? 0) + v;
    console.log(`  ${k}: ${v}`);
  }
}
console.log("\nTotals:");
for (const [k, v] of Object.entries(totals)) console.log(`  ${k}: ${v}`);
