// Surgical pass: in lines that contain BOTH `uppercase` in className AND a JSX
// expression rendering an entity-content variable, strip `uppercase` from the
// className. This stops file paths, symbol names, error messages, etc. from
// being force-capped.
//
// We only act when the line carrying `uppercase` and the line carrying `>{var}<`
// are within ±5 lines of each other AND the variable matches one of the known
// entity-content names below. Kind/visibility/language tags are NOT in this
// list — they keep `uppercase` (Role B short label, ≤14 chars).
import fs from "node:fs";
import path from "node:path";
import { srcRoot as root } from "./_src-root.mjs";


// Variable expressions that we know render multi-word prose, file paths, ids,
// names, descriptions — content where forced uppercase hurts legibility.
// Kept tight & specific to avoid stripping uppercase from short label tags.
const DROP_PATTERNS = [
  /\{error\}/,
  /\{toast\.detail\}/,
  /\{toast\.label\}/, // toast labels are short prose like "Indexing started"
  /\{cmd\.hint\}/,
  /\{description\}/,
  /\{symbol\.module_path\}/,
  /\{previewed\.module_path\}/,
  /\{result\.content_id\}/,
  /\{project\.name\}/,
  /\{def\.heading_path/,
  /\{section\.title\}/,
  /\{root\}/,
  /\{basename\.replace/,
  /\{group\.label\}/,
  /\{corpusLabel\(/,
  /\{name\}/, // CorpusChip name
  /\{labels\[/,
  /\{emptyLabel\}/,
  /\{children\}/, // Bridge SectionRow generic children
  /\{token\}/, // Settings token (likely lowercase id)
  /\{subtitle\}/, // EntityRow subtitle (path under name)
  /\{label\}/, // metric-tile / labeled-row callers
  /\{o\.label\}/, // Settings option label
  /\{symbols\.length === 0/, // SymbolGraph empty messages
  /\{refs\.length === 0/,
  /\{loading \? "loading_"/, // SymbolGraph empty/loading
  /\{loading \? "QUERYING/, // Bridge loading messages
  /\{t\.name\}/, // SymbolGraph hover symbol name
  /\{p\}/, // QueryPlayground recent probe
  /\{i \+ 1\}/, // numeric index — caps does nothing for digits
];

function walk(d) {
  return fs
    .readdirSync(d, { withFileTypes: true })
    .flatMap((e) =>
      e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)],
    );
}

const files = walk(root).filter((f) => /\.tsx$/.test(f));

let totalEdits = 0;
const editsByFile = {};

for (const f of files) {
  const before = fs.readFileSync(f, "utf8");
  const lines = before.split("\n");
  const out = [...lines];
  let edits = 0;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (!/\buppercase\b/.test(line)) continue;
    if (/\/\//.test(line.slice(0, line.indexOf("uppercase")))) continue;

    // Look at +1..+5 lines for the JSX expression body.
    const ctx = lines.slice(i, Math.min(lines.length, i + 6)).join("\n");
    if (!DROP_PATTERNS.some((re) => re.test(ctx))) continue;

    // Strip `uppercase` plus its trailing space from the className.
    const replaced = out[i]
      .replace(/\s+uppercase\b/, "")
      .replace(/\buppercase\s+/, "")
      .replace(/\buppercase\b/, "");
    if (replaced !== out[i]) {
      out[i] = replaced;
      edits++;
    }
  }

  if (edits) {
    fs.writeFileSync(f, out.join("\n"));
    totalEdits += edits;
    editsByFile[path.relative(root, f).replaceAll("\\", "/")] = edits;
  }
}

for (const [file, n] of Object.entries(editsByFile)) {
  console.log(`${file}: -${n} uppercase`);
}
console.log(`total dropped: ${totalEdits}`);
