// Drop input/textarea/kbd from `border-2` to `border` (1px). 2px borders
// were universal before; in the field-manual palette controls get hairline
// borders and reserve 2px for active selection.
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

const files = walk(root).filter((f) => /\.tsx$/.test(f));

let total = 0;
const byFile = {};

for (const f of files) {
  const before = fs.readFileSync(f, "utf8");
  const lines = before.split("\n");
  const out = [...lines];
  let n = 0;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (!/border-2 border-border\b/.test(line)) continue;

    // Look at line + previous-2 lines for a JSX tag we want to soften.
    const ctx = lines.slice(Math.max(0, i - 2), i + 1).join("\n");
    const isInput = /<input\b/.test(ctx);
    const isKbd = /<kbd\b/.test(ctx);
    const isTextarea = /<textarea\b/.test(ctx);
    if (!isInput && !isKbd && !isTextarea) continue;

    out[i] = line.replace(/\bborder-2 border-border\b/g, "border border-border");
    if (out[i] !== line) n++;
  }

  if (n) {
    fs.writeFileSync(f, out.join("\n"));
    byFile[path.relative(root, f).replaceAll("\\", "/")] = n;
    total += n;
  }
}

for (const [file, n] of Object.entries(byFile)) console.log(`${file}: ${n}`);
console.log("total:", total);
