// One-shot helper: print every line containing text-[0.625rem].
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
for (const f of files) {
  const lines = fs.readFileSync(f, "utf8").split("\n");
  const rel = path.relative(root, f).replaceAll("\\", "/");
  for (let i = 0; i < lines.length; i++) {
    if (/text-\[0\.625rem\]/.test(lines[i])) {
      console.log(`${rel}:${i + 1}  ${lines[i].trim().slice(0, 140)}`);
    }
  }
}
