// One-off: convert hardcoded `text-[Npx]` Tailwind arbitrary classes to
// rem-based equivalents so they scale with the html font-size that the
// viewport-based media queries set.
import fs from "node:fs";
import path from "node:path";
import { srcRoot as root } from "./_src-root.mjs";


function walk(d) {
  return fs
    .readdirSync(d, { withFileTypes: true })
    .flatMap((e) =>
      e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)],
    );
}

// Map: px → rem-string. Anchor is 16px = 1rem; we accept that on the
// 14px-default-root our 10px sites land at 8.75px, slightly smaller —
// the trade-off is they grow at 4K to ~14px. Net win at scale.
const PX_TO_REM = {
  "9px": "0.5625rem",
  "10px": "0.625rem",
  "11px": "0.6875rem",
  "12px": "0.75rem",
  "13px": "0.8125rem",
  "14px": "0.875rem",
  "15px": "0.9375rem",
  "16px": "1rem",
};

const files = walk(root).filter(
  (f) => f.endsWith(".tsx") || f.endsWith(".ts"),
);

let totalEdits = 0;
for (const f of files) {
  let src = fs.readFileSync(f, "utf8");
  let edits = 0;

  for (const [px, rem] of Object.entries(PX_TO_REM)) {
    // text-[9px] → text-[0.5625rem]
    const re = new RegExp(`text-\\[${px.replace(/[.]/g, "\\.")}\\]`, "g");
    src = src.replace(re, () => {
      edits++;
      return `text-[${rem}]`;
    });
  }

  if (edits) {
    fs.writeFileSync(f, src);
    totalEdits += edits;
    console.log(path.relative(root, f).replaceAll("\\", "/") + ": +" + edits);
  }
}
console.log("total: " + totalEdits);
