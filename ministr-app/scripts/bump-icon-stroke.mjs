// One-off: bump strokeWidth=2.5 on every lucide-react icon JSX usage.
// Skips icons that already have a strokeWidth attribute.
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

const files = walk(root).filter(
  (f) => f.endsWith(".tsx") || f.endsWith(".ts"),
);

let totalEdits = 0;
for (const f of files) {
  let src = fs.readFileSync(f, "utf8");
  const imp = src.match(/import\s*\{([^}]+)\}\s*from\s*["']lucide-react["']/);
  if (!imp) continue;
  const icons = imp[1]
    .split(",")
    .map((s) => {
      const t = s.trim();
      const m = t.match(/^(\w+)(?:\s+as\s+(\w+))?$/);
      return m ? m[2] || m[1] : null;
    })
    .filter(Boolean);
  if (icons.length === 0) continue;

  let edits = 0;
  for (const ic of icons) {
    // Match opening JSX tag: <Icon ... > or <Icon ... />
    // Use [\s\S] to span multiline attribute lists.
    const re = new RegExp("<" + ic + "(\\s[\\s\\S]*?)?(\\/?)>", "g");
    src = src.replace(re, (m, attrs, slash) => {
      if (/strokeWidth/.test(m)) return m;
      const a = attrs || " ";
      edits++;
      return "<" + ic + a + (a.endsWith(" ") ? "" : " ") + "strokeWidth={2.5}" + slash + ">";
    });
  }

  if (edits) {
    fs.writeFileSync(f, src);
    totalEdits += edits;
    console.log(path.relative(root, f).replaceAll("\\", "/") + ": +" + edits);
  }
}
console.log("total: " + totalEdits);
