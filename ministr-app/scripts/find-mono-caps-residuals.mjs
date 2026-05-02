// Find every remaining `font-mono ... uppercase ... tracking-[0.05em]` that
// renders as a label/heading. We're looking specifically for Role-A or
// Role-C miscategorizations: long prose, multi-word headings, or text
// inside a section header that was missed by prior sweeps.
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

const findings = [];
for (const f of files) {
  const lines = fs.readFileSync(f, "utf8").split("\n");
  const rel = path.relative(root, f).replaceAll("\\", "/");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (!/font-mono[^"]*uppercase[^"]*tracking-/.test(line)) continue;
    if (/\/\//.test(line.slice(0, line.indexOf("font-mono")))) continue;
    // peek next 3 lines for the rendered text body
    const ctx = lines.slice(i, Math.min(lines.length, i + 4)).join("\n");
    const textMatch = ctx.match(/>\s*([^<>{}]{4,80})\s*</);
    if (!textMatch) continue;
    const text = textMatch[1].replace(/\s+/g, " ").trim();
    if (!text || /[a-z]/.test(text)) continue; // already mixed-case
    if (text.length <= 14 && !text.includes(" ")) continue; // Role-B short label
    findings.push({ file: rel, line: i + 1, text });
  }
}

for (const x of findings) {
  console.log(`${x.file}:${x.line}  "${x.text}"`);
}
console.log(`total: ${findings.length}`);
