// Sweep page-title `<h1|h2|h3>` whose className uses `font-sans text-lg`
// (post-legibility-pass survivors) → Plex Serif display.
// Also catches `font-sans text-xl/2xl ...` for the Onboarding hero.
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

const hits = {};

for (const f of files) {
  const before = fs.readFileSync(f, "utf8");
  let src = before;
  let n = 0;

  // <h2|h3 className="...font-sans text-{lg|xl|2xl} font-bold tracking-[Xem] text-text...">
  src = src.replace(
    /<(h[1-3])\s+className="([^"]*?)\bfont-sans\b([^"]*?)\bfont-bold\b([^"]*?)\btext-text\b([^"]*)">/g,
    (m, tag, a, b, c, d) => {
      const layout = [a, b, c, d].join(" ");
      // Bail if it's clearly not a heading (no text-{lg|xl|2xl}).
      if (!/\btext-(lg|xl|2xl|3xl)\b/.test(layout)) return m;
      n++;
      const cleaned = layout
        .split(/\s+/)
        .filter(
          (cls) =>
            !/^font-/.test(cls) &&
            !/^tracking-/.test(cls) &&
            !/^text-(lg|xl|2xl|3xl|sm|base|xs|\[)/.test(cls) &&
            cls !== "uppercase" &&
            cls !== "",
        )
        .join(" ");
      return `<${tag} className="font-serif text-2xl font-normal text-text leading-tight ${cleaned}">`;
    },
  );

  // Adjacent paragraph descriptor below page title — `font-sans text-xs ... text-text-dim mt-1`
  // Bump to body-muted serif italic? Plan calls for body Plex Sans for descriptions.
  // Just keep these as-is; they're already sans sentence case.

  if (n) {
    fs.writeFileSync(f, src);
    hits[path.relative(root, f).replaceAll("\\", "/")] = n;
  }
}

for (const [file, n] of Object.entries(hits)) console.log(`${file}: ${n}`);
console.log("total:", Object.values(hits).reduce((s, x) => s + x, 0));
