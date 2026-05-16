/**
 * One-shot codemod: migrate residual pre-Cockpit literals to the
 * contract (see DESIGN.md). Safe global string/regex replacements only.
 * Semantic edge cases (dead .motion-data/.ministr-pin-in in AskCitation)
 * are hand-fixed separately.
 */
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..", "src");

/** [regex, replacement] applied in order to every .ts/.tsx file. */
const RULES = [
  [/tracking-\[0\.05em\]/g, "tracking-[0.08em]"],
  [/tracking-\[0\.1em\]/g, "tracking-[0.08em]"],
  [/ministr-flash/g, "ministr-pulse"],
  [/transition-none/g, "transition-colors duration-150 ease-out"],
  [/\bfont-serif\b/g, "font-sans"],
  // hairline: border-2 → border (won't touch border-{x,y,l,r,t,b}-2)
  [/\bborder-2\b/g, "border"],
  [/\brounded-sm\b/g, "rounded-md"],
];

let files = 0;
let edits = 0;

function walk(dir) {
  for (const name of fs.readdirSync(dir)) {
    const fp = path.join(dir, name);
    const st = fs.statSync(fp);
    if (st.isDirectory()) walk(fp);
    else if (name.endsWith(".tsx") || name.endsWith(".ts")) {
      let src = fs.readFileSync(fp, "utf8");
      const before = src;
      for (const [re, rep] of RULES) src = src.replace(re, rep);
      if (src !== before) {
        fs.writeFileSync(fp, src);
        files++;
        const m = before.match(
          /tracking-\[0\.0?1?5?em\]|ministr-flash|transition-none|font-serif|border-2\b|rounded-sm/g,
        );
        edits += m ? m.length : 0;
      }
    }
  }
}

walk(ROOT);
console.log(`codemod: ${files} files changed, ~${edits} literals migrated`);
