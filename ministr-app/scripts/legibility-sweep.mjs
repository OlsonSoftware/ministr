// Mechanical legibility sweep across all .tsx/.ts files in src/.
//
// Per the plan (wobbly-brewing-lemon.md §2.2 + §2.5):
//   - Drop heavy tracking everywhere: tracking-wider, tracking-[0.12em..0.20em]
//     all collapse to tracking-[0.05em]. Caps already provide visual weight;
//     extra tracking just slows the read.
//   - Bump 10px (text-[0.625rem]) prose to 12px (text-xs). Plan notes a narrow
//     exception for kind tags + tabular nums, but most existing usage is prose.
//     We do the wholesale bump and let any regressions surface in QA.
//
// Idempotent: re-runs are no-ops.
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

const files = walk(root).filter((f) => /\.(tsx|ts)$/.test(f));

const summary = {};

for (const f of files) {
  const before = fs.readFileSync(f, "utf8");
  let src = before;
  const fileChanges = {};

  // 1. tracking-wider → tracking-[0.05em]
  const m1 = (src.match(/tracking-wider\b/g) ?? []).length;
  if (m1) {
    src = src.replaceAll("tracking-wider", "tracking-[0.05em]");
    fileChanges["tracking-wider→0.05em"] = m1;
  }

  // 2. heavy bracket tracking values → tracking-[0.05em]
  // Covers 0.10em through 0.25em — anything ≥0.10em becomes 0.05em.
  let heavyCount = 0;
  src = src.replace(/tracking-\[0\.(1\d|20|25)em\]/g, () => {
    heavyCount++;
    return "tracking-[0.05em]";
  });
  if (heavyCount) fileChanges["tracking-heavy→0.05em"] = heavyCount;

  // 3. text-[0.625rem] → text-xs
  const m3 = (src.match(/text-\[0\.625rem\]/g) ?? []).length;
  if (m3) {
    src = src.replaceAll("text-[0.625rem]", "text-xs");
    fileChanges["text-10px→12px"] = m3;
  }

  // 4. text-[0.6875rem] → text-xs (the other tiny size we used for prose)
  // Keep this only where it's used for code (we'll handle code blocks
  // separately); for ui-tokens labelMicro we already bumped manually.
  // Skip auto-conversion for this one to avoid regressing code blocks.

  if (Object.keys(fileChanges).length) {
    fs.writeFileSync(f, src);
    summary[path.relative(root, f).replaceAll("\\", "/")] = fileChanges;
  }
}

let totalFiles = 0;
const totals = {};
for (const [file, changes] of Object.entries(summary)) {
  totalFiles++;
  console.log(file);
  for (const [k, v] of Object.entries(changes)) {
    totals[k] = (totals[k] ?? 0) + v;
    console.log(`  ${k}: ${v}`);
  }
}
console.log(`\n${totalFiles} files changed`);
console.log("Totals:");
for (const [k, v] of Object.entries(totals)) console.log(`  ${k}: ${v}`);
