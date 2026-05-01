// Inventory hard-offset-shadow usage so the field-manual sweep can keep
// shadows ONLY where they signal focus / active state.
//
// Reports four buckets:
//   A. shadow-[var(--shadow-*)] (Tailwind arbitrary value)
//   B. shadow-brutal-* (legacy Tailwind class)
//   C. style={{ boxShadow }} inline
//   D. boxShadow assignments in any other code (CSS-in-JS objects)
//
// For each hit we capture the file, line, and ~80 chars of context so we
// can decide: keep (focused/active) vs remove.
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

const files = walk(root).filter((f) => /\.(tsx?|css)$/.test(f));

const A = [];
const B = [];
const C = [];

for (const f of files) {
  const lines = fs.readFileSync(f, "utf8").split("\n");
  const rel = path.relative(root, f).replaceAll("\\", "/");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (/shadow-\[var\(--shadow-/.test(line)) {
      A.push({ file: rel, line: i + 1, ctx: line.trim().slice(0, 100) });
    }
    if (/\bshadow-brutal-/.test(line)) {
      B.push({ file: rel, line: i + 1, ctx: line.trim().slice(0, 100) });
    }
    if (/\bboxShadow\b/.test(line)) {
      C.push({ file: rel, line: i + 1, ctx: line.trim().slice(0, 100) });
    }
  }
}

console.log("=== A. shadow-[var(--shadow-*)] ===");
for (const x of A) console.log(`${x.file}:${x.line}  ${x.ctx}`);
console.log(`  total: ${A.length}\n`);

console.log("=== B. shadow-brutal-* ===");
for (const x of B) console.log(`${x.file}:${x.line}  ${x.ctx}`);
console.log(`  total: ${B.length}\n`);

console.log("=== C. boxShadow inline ===");
for (const x of C) console.log(`${x.file}:${x.line}  ${x.ctx}`);
console.log(`  total: ${C.length}`);
