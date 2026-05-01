// Inventory accent usage. After Phase 2 the accent reserves to:
//   1. focus-visible outline
//   2. active/selected state (one per view)
//   3. live cursor
//   4. wordmark underbar
//   5. live-data signaling
//
// Anything else — bg-accent on chips, border-accent on idle controls,
// text-accent on labels — should drop.
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

const tokens = ["bg-accent", "border-accent", "text-accent", "ring-accent"];
const counts = Object.fromEntries(tokens.map((t) => [t, 0]));
const findingsByFile = {};

for (const f of files) {
  const lines = fs.readFileSync(f, "utf8").split("\n");
  const rel = path.relative(root, f).replaceAll("\\", "/");
  let n = 0;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    for (const t of tokens) {
      // Use a word-boundary-ish check that avoids `bg-accent-fg-on` etc.
      const re = new RegExp(`\\b${t}\\b(?!-)`);
      if (re.test(line)) {
        counts[t]++;
        (findingsByFile[rel] ??= []).push({ line: i + 1, token: t, ctx: line.trim().slice(0, 100) });
      }
    }
  }
  void n;
}

console.log("=== Counts ===");
for (const [t, n] of Object.entries(counts)) console.log(`  ${t}: ${n}`);

console.log("\n=== Per-file (top 30) ===");
let printed = 0;
for (const [file, hits] of Object.entries(findingsByFile)) {
  if (printed >= 30) break;
  console.log(`\n${file}  (${hits.length})`);
  for (const h of hits) {
    if (printed++ >= 30) break;
    console.log(`  :${h.line}  [${h.token}]  ${h.ctx}`);
  }
}
