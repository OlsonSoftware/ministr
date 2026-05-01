// Fix lines where multi-word prose is being force-capped via className.
//
// Two cases handled:
//
// CASE 1 — Text already sentence-case, but className applies `font-mono
//   uppercase ...`. The CSS turns it into ALL CAPS. Drop `uppercase`, switch
//   `font-mono` → `font-sans`. Keeps existing tracking, weight, color.
//
// CASE 2 — Text is hardcoded ALL CAPS and >14 chars (Role C body, not a
//   label). Convert to sentence case AND drop `uppercase` AND swap the font.
//
// We only act when className appears on the same line as `uppercase` AND a
// rendered text body lives within the next 4 lines (matches our JSX style of
// `<span className="...">\n  body\n</span>`).
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

function toSentence(s) {
  // Preserve a few well-known acronyms that the lowercaser would otherwise
  // destroy. Replace them with placeholders, lowercase, then restore.
  const ACR = ["LLM", "API", "HTTP", "HTTPS", "IPC", "FFI", "URL", "IDE", "CLI", "NAPI", "JSON", "XML", "SQL", "UI", "OS", "PYO3", "WASM", "TLS", "SSL", "UDS", "HNSW", "ONNX", "UUID", "TS", "JS", "RS", "PY"];
  const tokens = s.split(/(\s+)/);
  const lowered = tokens
    .map((tok) => {
      if (/^\s+$/.test(tok)) return tok;
      // Look at the bare word (strip trailing punct).
      const m = tok.match(/^([A-Z0-9⌘·.,!?:;\-/&()'"’]+?)([.,!?:;\-)]*)$/);
      if (!m) return tok.toLowerCase();
      const word = m[1];
      const trailing = m[2];
      if (ACR.includes(word.toUpperCase()) && word === word.toUpperCase()) {
        return word + trailing; // keep acronym
      }
      return word.toLowerCase() + trailing;
    })
    .join("");
  // Capitalize first letter of the whole string.
  const idx = lowered.search(/[a-zA-Z]/);
  if (idx < 0) return lowered;
  return lowered.slice(0, idx) + lowered[idx].toUpperCase() + lowered.slice(idx + 1);
}

const files = walk(root).filter((f) => /\.tsx$/.test(f));

let case1 = 0;
let case2 = 0;
const editsByFile = {};

for (const f of files) {
  const src = fs.readFileSync(f, "utf8");
  const lines = src.split("\n");
  const out = [...lines];
  let fileEdits = 0;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (!/\bfont-mono\b/.test(line)) continue;
    if (!/\buppercase\b/.test(line)) continue;
    if (/\/\//.test(line.slice(0, line.indexOf("uppercase")))) continue;

    // Find rendered text in i..i+4. Two patterns:
    //   - same-line: >TEXT<
    //   - multi-line: opening tag this line, body next line, closing line later
    const window = lines.slice(i, Math.min(lines.length, i + 5));
    const blob = window.join("\n");

    // Match a body that contains real letters. We match either >...< text
    // node OR the text node on the line after the opening >.
    const textMatch = blob.match(/>\s*([^<>{}]+?)\s*</);
    if (!textMatch) continue;
    const text = textMatch[1].replace(/\s+/g, " ").trim();
    if (!text) continue;

    // Is the text essentially prose? Need at least one space (multi-word) AND
    // not be a punctuation-only chunk.
    if (!/\s/.test(text)) continue;
    if (!/[A-Za-z]/.test(text)) continue;
    // Skip text that is itself a JSX expression result (we can't tell shape).
    if (/^[\{\}]/.test(text)) continue;

    const allCaps = !/[a-z]/.test(text) && /[A-Z]/.test(text);
    const hasLower = /[a-z]/.test(text);

    if (allCaps && text.length <= 14) continue; // Role B short label, keep
    if (!allCaps && !hasLower) continue;

    // Apply edit to this line: drop `uppercase`, swap font-mono → font-sans.
    // Keep tracking value as-is (already 0.05em from the prior sweep).
    let edited = out[i]
      .replace(/\s+uppercase\b/, "")
      .replace(/\buppercase\s+/, "")
      .replace(/\buppercase\b/, "")
      .replace(/\bfont-mono\b/, "font-sans");

    // For CASE 2 (all-caps text), also convert the text in subsequent lines.
    if (allCaps) {
      const sentence = toSentence(text);
      // Walk forward to find which line holds the captured text and replace it.
      for (let j = 0; j < window.length; j++) {
        const wline = window[j];
        if (wline.includes(text)) {
          out[i + j] = out[i + j].replace(text, sentence);
          break;
        }
        // Multi-line text won't match exactly; try the per-line component.
        const stripped = wline.trim();
        if (stripped && text.startsWith(stripped) && stripped.length > 6) {
          const partSentence = toSentence(stripped);
          out[i + j] = out[i + j].replace(stripped, partSentence);
        }
      }
      case2++;
    } else {
      case1++;
    }

    if (edited !== out[i]) {
      out[i] = edited;
      fileEdits++;
    }
  }

  if (fileEdits) {
    fs.writeFileSync(f, out.join("\n"));
    editsByFile[path.relative(root, f).replaceAll("\\", "/")] = fileEdits;
  }
}

for (const [file, n] of Object.entries(editsByFile)) {
  console.log(`${file}: ${n}`);
}
console.log(`\nCASE 1 (mixed-case text, drop CSS uppercase): ${case1}`);
console.log(`CASE 2 (all-caps prose, convert + drop): ${case2}`);
