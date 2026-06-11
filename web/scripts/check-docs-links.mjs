// Validates internal /docs/* links: every link target in content/docs mdx and in
// app/components source must resolve to a page in content/docs.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const docsDir = join(root, 'content/docs');

function walk(dir, exts) {
  const out = [];
  for (const name of readdirSync(dir)) {
    if (name === 'node_modules' || name === 'out' || name.startsWith('.')) continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...walk(p, exts));
    else if (exts.some((e) => name.endsWith(e))) out.push(p);
  }
  return out;
}

// Build the set of valid /docs slugs from the content tree.
const slugs = new Set(['/docs']);
for (const f of walk(docsDir, ['.mdx'])) {
  let rel = relative(docsDir, f).replace(/\.mdx$/, '');
  if (rel.endsWith('/index') || rel === 'index') rel = rel.replace(/\/?index$/, '');
  slugs.add(rel ? `/docs/${rel}` : '/docs');
}

const linkRe = /\((\/docs[^)#\s]*)(?:#[^)\s]*)?\)|["'`](\/docs[^"'`#\s]*)(?:#[^"'`\s]*)?["'`]/g;
const sources = [
  ...walk(docsDir, ['.mdx', '.json']),
  ...walk(join(root, 'app'), ['.tsx', '.ts']),
  ...walk(join(root, 'components'), ['.tsx', '.ts']),
];

let bad = 0;
for (const f of sources) {
  const text = readFileSync(f, 'utf8');
  for (const m of text.matchAll(linkRe)) {
    const link = (m[1] ?? m[2]).replace(/\/$/, '');
    if (link.includes('$') || link.includes('[')) continue; // route/template literals
    if (!slugs.has(link)) {
      console.error(`BROKEN ${link}  in ${relative(root, f)}`);
      bad++;
    }
  }
}
console.log(`${slugs.size} pages, ${bad} broken /docs links`);
process.exit(bad ? 1 : 0);
