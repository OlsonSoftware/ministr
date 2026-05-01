// Find `uppercase` className usage and the rendered content next to it.
// Goal: identify entity-name renders (file paths, symbol names, session ids,
// corpus names) where forced uppercase hides letter shape.
//
// Output groups:
//   A. JSX expression content like {x.path} — likely entity name → drop uppercase
//   B. Static string content (label) — keep uppercase if ≤14 chars (Role B)
//   C. Mapped variable like {item.kind} — likely tag → keep
//
// We print the line + a context snippet so a human can decide.
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

const files = walk(root).filter((f) => /\.tsx?$/.test(f));

const A = [];
const B = [];
const C = [];

for (const f of files) {
  const src = fs.readFileSync(f, "utf8");
  const rel = path.relative(root, f).replaceAll("\\", "/");
  const lines = src.split("\n");

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (!/\buppercase\b/.test(line)) continue;
    if (/\/\//.test(line.slice(0, line.indexOf("uppercase")))) continue;

    // Look at the next ~5 lines to find the rendered content of this element.
    const ctx = lines.slice(i, Math.min(lines.length, i + 6)).join("\n");

    // Heuristic: detect JSX expression content vs static.
    // Looking for >{ ... }<  near here.
    const exprMatch = ctx.match(/>\s*\{([^}]{1,80})\}\s*</);
    const staticMatch = ctx.match(/>\s*([A-Z][^<>{]{0,80})\s*</);

    if (exprMatch) {
      const expr = exprMatch[1].trim();
      // Skip if the expr contains .toUpperCase() — author already CHOSE to caps it.
      if (/\.toUpperCase\(/.test(expr)) {
        C.push({ file: rel, line: i + 1, expr, ctx: ctx.slice(0, 200) });
      } else {
        A.push({ file: rel, line: i + 1, expr, ctx: ctx.slice(0, 200) });
      }
    } else if (staticMatch) {
      B.push({
        file: rel,
        line: i + 1,
        text: staticMatch[1].trim(),
        ctx: ctx.slice(0, 200),
      });
    }
  }
}

console.log("=== A. uppercase + JSX expression (likely entity name) ===");
for (const x of A) {
  console.log(`${x.file}:${x.line}  {${x.expr}}`);
}
console.log(`\n  Total A: ${A.length}\n`);

console.log("=== B. uppercase + static string (Role B labels — review by length) ===");
for (const x of B.slice(0, 60)) {
  const flag = x.text.length > 14 ? "  TOO LONG" : "";
  console.log(`${x.file}:${x.line}  "${x.text}" (${x.text.length}ch)${flag}`);
}
console.log(`\n  Total B: ${B.length}`);

console.log(`\n=== C. uppercase + .toUpperCase() (already-converted tags — keep) ===`);
console.log(`  Total C: ${C.length}`);
