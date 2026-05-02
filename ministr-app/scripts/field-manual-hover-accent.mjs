// Drop accent from idle hover/focus states. Active-state ternaries
// (`active ? "bg-accent..." : "..."`) keep their accent — that's the
// scarce signal we're preserving. We only touch:
//   - `hover:bg-accent hover:text-[var(--color-accent-fg-on)]` (and adjacent)
//   - `focus:bg-accent focus:text-[var(--color-accent-fg-on)]`
//   - `bg-accent/15` (semi-transparent accent backgrounds)
//
// The replacement is a quiet surface-overlay treatment so the eye still
// gets a hover cue without the page lighting up purple.
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

const files = walk(root).filter((f) => /\.tsx$/.test(f));

let total = 0;
const byFile = {};

for (const f of files) {
  const before = fs.readFileSync(f, "utf8");
  let src = before;
  let n = 0;

  // hover:bg-accent hover:text-[var(--color-accent-fg-on)] → surface-overlay/text
  src = src.replace(
    /hover:bg-accent hover:text-\[var\(--color-accent-fg-on\)\]/g,
    () => {
      n++;
      return "hover:bg-surface-overlay hover:text-text";
    },
  );

  // focus:bg-accent focus:text-[var(--color-accent-fg-on)] (inputs)
  src = src.replace(
    /focus:bg-accent focus:text-\[var\(--color-accent-fg-on\)\]/g,
    () => {
      n++;
      return "focus:bg-surface-overlay focus:text-text";
    },
  );

  // focus:placeholder:text-[var(--color-accent-fg-on)] paired with the above —
  // strip; the placeholder color stays as-is on focus.
  src = src.replace(
    /\s+focus:placeholder:text-\[var\(--color-accent-fg-on\)\]/g,
    () => {
      n++;
      return "";
    },
  );

  // bg-accent/15 → bg-surface-overlay
  src = src.replace(/\bbg-accent\/15\b/g, () => {
    n++;
    return "bg-surface-overlay";
  });

  // hover:bg-accent (without paired text) — bare hover-fill on idle controls.
  // Also strip — replace with surface-overlay.
  src = src.replace(/(?<!:)\bhover:bg-accent\b(?!-)/g, () => {
    n++;
    return "hover:bg-surface-overlay";
  });

  if (n) {
    fs.writeFileSync(f, src);
    byFile[path.relative(root, f).replaceAll("\\", "/")] = n;
    total += n;
  }
}

for (const [file, n] of Object.entries(byFile)) console.log(`${file}: ${n}`);
console.log("total:", total);
