#!/usr/bin/env node
/**
 * design-lint — the v5 "Clear Glass" guardrail (rules from ../DESIGN.md v5
 * §9 — the mechanized half of the Definition of Done).
 *
 * Fails (exit 1) if a banned literal reappears in a className. Comments and
 * the tokens/contract files are excluded so the canonical definitions and
 * prose can name the banned strings. Each rule below cites its DESIGN.md §.
 *
 * Run: `npm run design:lint` (also wired into `just validate`).
 */
const fs = require("fs");
const path = require("path");

const SRC = path.join(__dirname, "..", "src");

// Files allowed to mention banned strings (they define / document them).
const ALLOW = new Set(["components/ui/trust.ts", "main.tsx"]);

// [label, regex] — regex runs against comment-stripped source.
const BANNED = [
  // --- Consistency floor (DESIGN.md §1/§6/§7/§8): inherited denylist ---
  ["tracking-[0.05em] (§6 — use [0.06em]/[0.08em])", /tracking-\[0\.05em\]/],
  ["tracking-[0.1em] (§6 — use [0.06em]/[0.08em])", /tracking-\[0\.1em\]/],
  ["transition-none (§8 — clickable things must animate)", /\btransition-none\b/],
  ["rounded-none (§7 — even data surfaces are rounded)", /\brounded-none\b/],
  ["border-2 (§4 — hairline borders only)", /\bborder-2\b/],
  ["font-serif (§6 — Geist sans / JetBrains mono only)", /\bfont-serif\b/],
  ["italic (§6 — use text-text-dim, not italic)", /(?<!not-)(?<!\w)italic(?!\w)/],
  ["ministr-flash (dead class)", /\bministr-flash\b/],
  ["motion-data (dead class)", /["'\s]motion-data["'\s]/],
  ["ministr-pin-in (dead class)", /\bministr-pin-in\b/],
  ["ministr-drawer-in (dead class)", /\bministr-drawer-in\b/],
  // --- §4 Elevation: shadows go through the scale; only the glow is arbitrary
  ["arbitrary shadow (§4 — use shadow-xs…lg)", /shadow-\[(?!var\(--glow-soft\)\])/],
  // --- §3 Color: token-only color — no raw hex / rgb / hsl in a className
  ["raw hex color (§3 — token-only color)", /-\[#[0-9a-fA-F]{3,8}\b/],
  ["raw rgb/hsl color (§3 — token-only color)", /-\[(?:rgb|hsl)a?\(/],
  // --- §4 Glass: backdrop-blur belongs to the glass role token, not ad-hoc
  ["arbitrary backdrop-blur (§4 — use the glass token)", /\bbackdrop-blur-\[/],
  // --- §8 Motion: durations go through swift/flow/spring tokens
  ["arbitrary duration (§8 — use a motion token)", /\bduration-\[/],
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
console.log("design-lint: clean — UI is on the Clear Glass contract.");
