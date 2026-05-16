#!/usr/bin/env node
/**
 * design-lint — the Cockpit consistency guardrail (see ../DESIGN.md).
 *
 * Fails (exit 1) if any banned pre-Cockpit literal reappears in a
 * className. Comments and the tokens/contract files are excluded so the
 * canonical definitions and prose can name the banned strings.
 *
 * Run: `pnpm design:lint` (also wired into `just validate`).
 */
const fs = require("fs");
const path = require("path");

const SRC = path.join(__dirname, "..", "src");

// Files allowed to mention banned strings (they define / document them).
const ALLOW = new Set(["lib/ui-tokens.ts", "lib/motion.ts"]);

// [label, regex] — regex runs against comment-stripped source.
const BANNED = [
  ["tracking-[0.05em]", /tracking-\[0\.05em\]/],
  ["tracking-[0.1em]", /tracking-\[0\.1em\]/],
  ["transition-none", /\btransition-none\b/],
  ["rounded-none", /\brounded-none\b/],
  ["rounded-sm", /\brounded-sm\b/],
  ["border-2", /\bborder-2\b/],
  ["font-serif", /\bfont-serif\b/],
  ["ministr-flash", /\bministr-flash\b/],
  ["motion-data (dead class)", /["'\s]motion-data["'\s]/],
  ["ministr-pin-in (dead class)", /\bministr-pin-in\b/],
  ["ministr-drawer-in (dead class)", /\bministr-drawer-in\b/],
  // arbitrary shadow except the sanctioned glow
  ["arbitrary shadow", /shadow-\[(?!var\(--glow-soft\)\])/],
];

/** Strip // line and /* *\/ block comments (keeps string contents). */
function stripComments(s) {
  return s
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/(^|[^:])\/\/[^\n]*/g, "$1");
}

let violations = 0;
function walk(dir) {
  for (const name of fs.readdirSync(dir)) {
    const fp = path.join(dir, name);
    const st = fs.statSync(fp);
    if (st.isDirectory()) walk(fp);
    else if (name.endsWith(".tsx") || name.endsWith(".ts")) {
      const rel = path.relative(SRC, fp).replace(/\\/g, "/");
      if (ALLOW.has(rel)) continue;
      const code = stripComments(fs.readFileSync(fp, "utf8"));
      for (const [label, re] of BANNED) {
        if (re.test(code)) {
          console.error(`  ✗ ${rel}: ${label}`);
          violations++;
        }
      }
    }
  }
}

walk(SRC);

if (violations > 0) {
  console.error(
    `\ndesign-lint: ${violations} consistency violation(s). ` +
      `Use a primitive in components/ui/* or a role token in ` +
      `lib/ui-tokens.ts (see ministr-app/DESIGN.md).`,
  );
  process.exit(1);
}
console.log("design-lint: clean — UI is on the Cockpit contract.");
