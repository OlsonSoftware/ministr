// Audit remaining legibility issues after the caps-to-sentence sweep:
// 1. Broken acronyms inside converted prose (e.g. "llm", "api", "http", "url").
// 2. Heavy tracking values that should be reduced (`tracking-[0.18em]`,
//    `tracking-[0.15em]`, `tracking-wider`).
// 3. Tiny prose text (`text-[0.625rem]`) that should be `text-xs`.
// 4. Sans-serif uppercase usage (Role-violation per §2.8).
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

const ACRONYMS = [
  "llm",
  "api",
  "http",
  "https",
  "ipc",
  "ffi",
  "url",
  "ide",
  "cli",
  "napi",
  "css",
  "html",
  "json",
  "xml",
  "yaml",
  "toml",
  "sql",
  "ui",
  "os",
  "cpu",
  "gpu",
  "ram",
  "ssd",
  "pyo3",
  "tauri", // proper noun — ok lowercase, just flag
  "wasm",
  "ssh",
  "ftp",
  "tcp",
  "udp",
  "tls",
  "ssl",
  "uds",
  "hnsw",
  "onnx",
  "uuid",
];

const files = walk(root).filter((f) => /\.(tsx|ts)$/.test(f));

const findings = {};

for (const f of files) {
  const src = fs.readFileSync(f, "utf8");
  const rel = path.relative(root, f).replaceAll("\\", "/");
  const issues = [];

  // 1. Broken acronyms in JSX text — look for `>... <acronym> ...<` where
  //    surrounding text is sentence-cased.
  src.split("\n").forEach((line, i) => {
    // Skip import lines, type lines, comments.
    if (/^\s*(import|export|type|interface|\/\/|\*)/.test(line)) return;

    // Find > ... < text content
    const m = [...line.matchAll(/>([^<>{]{3,150})</g)];
    for (const [, text] of m) {
      const t = text.trim();
      if (!t) continue;
      // Look for lowercase acronym surrounded by word boundaries.
      for (const acr of ACRONYMS) {
        const re = new RegExp(`\\b${acr}\\b`);
        if (re.test(t)) {
          // Skip if it's part of a tag name (whole text is one acronym).
          if (t === acr) continue;
          // Skip cases where the acronym appears inside a JSX expression.
          issues.push({
            type: "acronym",
            line: i + 1,
            text: t.slice(0, 100),
            acr,
          });
          break;
        }
      }
    }

    // 2. Heavy tracking
    if (/tracking-\[0\.1[2-9]em\]/.test(line)) {
      issues.push({ type: "tracking-heavy", line: i + 1, text: line.trim().slice(0, 100) });
    }
    if (/tracking-wider/.test(line)) {
      issues.push({ type: "tracking-wider", line: i + 1, text: line.trim().slice(0, 100) });
    }

    // 3. Tiny prose
    if (/text-\[0\.625rem\]/.test(line)) {
      issues.push({ type: "text-10px", line: i + 1, text: line.trim().slice(0, 100) });
    }

    // 4. Sans uppercase (rare — sans + uppercase is the worst combo).
    if (/font-sans[^"']*\buppercase\b/.test(line) || /\buppercase[^"']*font-sans\b/.test(line)) {
      issues.push({ type: "sans-uppercase", line: i + 1, text: line.trim().slice(0, 100) });
    }
  });

  if (issues.length) {
    findings[rel] = issues;
  }
}

// Print summary by type.
const byType = {};
for (const [file, issues] of Object.entries(findings)) {
  for (const issue of issues) {
    byType[issue.type] = (byType[issue.type] ?? 0) + 1;
  }
}
console.log("=== Summary ===");
for (const [t, n] of Object.entries(byType)) console.log(`  ${t}: ${n}`);

// Top 25 acronym hits (the ones from the sweep most likely to be broken).
console.log("\n=== Acronym hits (first 30) ===");
let n = 0;
for (const [file, issues] of Object.entries(findings)) {
  for (const issue of issues) {
    if (issue.type !== "acronym") continue;
    if (n++ >= 30) break;
    console.log(`${file}:${issue.line}  [${issue.acr}]  ${issue.text}`);
  }
  if (n >= 30) break;
}

// Files with heavy tracking.
console.log("\n=== Files with tracking-heavy/wider ===");
const trackingFiles = new Set();
for (const [file, issues] of Object.entries(findings)) {
  if (issues.some((i) => i.type === "tracking-heavy" || i.type === "tracking-wider")) {
    trackingFiles.add(file);
  }
}
for (const f of trackingFiles) console.log(f);

// Files with text-10px.
console.log("\n=== Files with text-[0.625rem] ===");
const tinyFiles = new Set();
for (const [file, issues] of Object.entries(findings)) {
  if (issues.some((i) => i.type === "text-10px")) tinyFiles.add(file);
}
for (const f of tinyFiles) console.log(f);
