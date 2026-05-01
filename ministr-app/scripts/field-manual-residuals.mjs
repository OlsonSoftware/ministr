// Catch-all sweep for residual brutalist patterns the prior sweeps missed:
//
//   1. <h2 className="font-serif ...">  STRUCTURE  </h2>
//      Page-title text body still ALL-CAPS even though the className flipped
//      to serif. Convert the body text to sentence-case.
//
//   2. <h3 className="font-mono text-lg font-bold uppercase tracking-[0.05em] text-text">
//      Page subtitles still on mono-caps. Convert to Plex Serif chapter heading.
//
//   3. `font-mono ... uppercase tracking-[0.05em] text-text` heading-style
//      class strings used at the section-anchor scale (text-base / text-lg)
//      with multi-word body text → swap to font-serif.
//
//   4. `border-l-4 border-accent` (heavy left-rule accent on doc-blocks etc.)
//      → `border-l-2 border-border-soft` (hairline marginalia rule).
import fs from "node:fs";
import path from "node:path";

const root = "D:/Code/ministr/ministr-app/src";

const ACR = ["LLM", "API", "HTTP", "IPC", "FFI", "URL", "IDE", "CLI", "NAPI",
  "JSON", "XML", "SQL", "UI", "OS", "PYO3", "WASM", "TLS", "TS", "JS", "RS",
  "PY", "MCP", "ID", "AI"];

function toSentence(s) {
  const tokens = s.split(/(\s+)/);
  const lowered = tokens
    .map((tok) => {
      if (/^\s+$/.test(tok)) return tok;
      const m = tok.match(/^([A-Z0-9⌘·.,!?:;\-/&()'"’]+?)([.,!?:;\-)]*)$/);
      if (!m) return tok.toLowerCase();
      const word = m[1];
      const trail = m[2];
      if (ACR.includes(word.toUpperCase()) && word === word.toUpperCase()) {
        return word + trail;
      }
      return word.toLowerCase() + trail;
    })
    .join("");
  const idx = lowered.search(/[a-zA-Z]/);
  if (idx < 0) return lowered;
  return lowered.slice(0, idx) + lowered[idx].toUpperCase() + lowered.slice(idx + 1);
}

function walk(d) {
  return fs
    .readdirSync(d, { withFileTypes: true })
    .flatMap((e) =>
      e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)],
    );
}

const files = walk(root).filter((f) => /\.tsx$/.test(f));
let total = 0;
const byFile = {};

for (const f of files) {
  const before = fs.readFileSync(f, "utf8");
  let src = before;
  const stats = {};

  // (1) Serif heading whose text is ALL CAPS — convert body to sentence case.
  src = src.replace(
    /(<(h[1-3])\s+className="font-serif[^"]*">\s*)([^<>{}]+?)(\s*<\/\2>)/g,
    (m, open, _tag, body, close) => {
      const t = body.trim();
      if (!t) return m;
      if (/[a-z]/.test(t)) return m;
      if (!/[A-Z]/.test(t)) return m;
      stats["serif-heading-text→sentence"] =
        (stats["serif-heading-text→sentence"] ?? 0) + 1;
      return open + body.replace(t, toSentence(t)) + close;
    },
  );

  // (2) <h3 className="font-mono text-lg font-bold uppercase tracking-[Xem] text-text">
  src = src.replace(
    /<h3\s+className="font-mono text-(lg|base) font-bold uppercase tracking-\[[^\]]+\] text-text">\s*([^<>{}]+?)\s*<\/h3>/g,
    (m, _sz, body) => {
      stats["h3-mono-caps→serif"] = (stats["h3-mono-caps→serif"] ?? 0) + 1;
      const t = body.trim();
      const sentence = /[a-z]/.test(t) ? t : toSentence(t);
      return `<h3 className="font-serif text-lg font-bold text-text leading-snug">${sentence}</h3>`;
    },
  );

  // (3) Heavy left-accent rules on doc/marginalia blocks — soften.
  // border-l-4 border-accent → border-l-2 border-border-soft
  src = src.replace(/\bborder-l-4\b/g, () => {
    stats["border-l-4→l-2"] = (stats["border-l-4→l-2"] ?? 0) + 1;
    return "border-l-2";
  });

  // (4) Standalone .ministr-wordmark stays — no edit.

  if (Object.keys(stats).length) {
    fs.writeFileSync(f, src);
    byFile[path.relative(root, f).replaceAll("\\", "/")] = stats;
    total += Object.values(stats).reduce((s, x) => s + x, 0);
  }
}

for (const [file, stats] of Object.entries(byFile)) {
  console.log(file);
  for (const [k, v] of Object.entries(stats)) console.log(`  ${k}: ${v}`);
}
console.log("\ntotal edits:", total);
