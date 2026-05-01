// Inventory `border-2 border-border` usage. After Phase 2:
//   - 2px stays only for active/selected state and primary actions.
//   - everything else moves to `border border-border-soft` (1px hairline).
//
// We can't safely auto-fix without context (a `border-2` on an active row
// must stay), so this just produces a triage list grouped by file.
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

const findings = {};
let total = 0;
for (const f of files) {
  const lines = fs.readFileSync(f, "utf8").split("\n");
  const rel = path.relative(root, f).replaceAll("\\", "/");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (/\bborder-2\b/.test(line)) {
      (findings[rel] ??= []).push({ line: i + 1, ctx: line.trim().slice(0, 120) });
      total++;
    }
  }
}

for (const [file, hits] of Object.entries(findings)) {
  console.log(`\n${file}  (${hits.length})`);
  for (const h of hits) console.log(`  :${h.line}  ${h.ctx}`);
}
console.log(`\ntotal border-2 instances: ${total}`);
