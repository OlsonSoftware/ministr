// Tool-reference generator — docs read what agents read, by construction.
//
// Injects a generated block (canonical description + parameter table from
// the live tool schema) into each content/docs/tools/<slug>.mdx between
// @generated markers, sourced from content/tools-manifest.json (itself
// regenerated from Rust: `cargo run -p ministr-mcp --example tool_manifest`).
//
//   node scripts/gen-tool-docs.mjs           # write pages
//   node scripts/gen-tool-docs.mjs --check   # CI gate: fail on any drift
import { readFileSync, writeFileSync, existsSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const toolsDir = join(root, 'content/docs/tools');
const manifest = JSON.parse(readFileSync(join(root, 'content/tools-manifest.json'), 'utf8'));
const check = process.argv.includes('--check');

const START = '{/* @generated tool-docs start — do not edit; run `npm run docs:gen` */}';
const END = '{/* @generated tool-docs end */}';

const slugOf = (name) => name.replace(/^ministr_/, '').replace(/_/g, '-');

// Escape MDX-hostile characters in plain text (JSX braces/tags), but leave
// content inside backtick code spans untouched.
function mdx(text) {
  return text
    .split(/(`[^`]*`)/)
    .map((part, i) => (i % 2 ? part : part.replace(/[{}<>]/g, (c) => `\\${c}`)))
    .join('');
}

function typeOf(prop) {
  let t = prop.type;
  if (Array.isArray(t)) t = t.filter((x) => x !== 'null').join(' | ') || 'any';
  if (t === 'array') {
    const item = prop.items?.type;
    if (item) t = `array of ${Array.isArray(item) ? item.join('/') : item}`;
  }
  if (!t && prop.anyOf) t = prop.anyOf.map((v) => v.type ?? '…').filter((x) => x !== 'null').join(' | ');
  return t ?? 'any';
}

function blockFor(tool) {
  const schema = tool.input_schema ?? {};
  const props = schema.properties ?? {};
  const requiredList = new Set(schema.required ?? []);
  const lines = [START, ''];
  lines.push(`> ${mdx(tool.description ?? '')}`);
  lines.push('');
  lines.push('## Parameters');
  lines.push('');
  const names = Object.keys(props).sort();
  if (names.length === 0) {
    lines.push('None.');
  } else {
    lines.push('| Parameter | Type | Required | Description |');
    lines.push('|---|---|---|---|');
    for (const name of names) {
      const p = props[name];
      const nullable = Array.isArray(p.type) && p.type.includes('null');
      const required = requiredList.has(name) || (!nullable && p.default === undefined);
      const desc = mdx(p.description ?? '').replace(/\|/g, '\\|').replace(/\n+/g, ' ');
      lines.push(`| \`${name}\` | ${typeOf(p)} | ${required ? 'yes' : 'no'} | ${desc} |`);
    }
  }
  const a = tool.annotations ?? {};
  const hints = [];
  if (a.readOnlyHint) hints.push('read-only');
  if (a.destructiveHint) hints.push('destructive');
  if (a.idempotentHint) hints.push('idempotent');
  if (a.openWorldHint) hints.push('open-world');
  if (hints.length) {
    lines.push('');
    lines.push(`Annotations: ${hints.join(' · ')}.`);
  }
  lines.push('');
  lines.push(
    '<small>This block is generated from the live tool schema — the same definition agents receive.</small>',
  );
  lines.push('', END);
  return lines.join('\n');
}

// Replace the marker block, or on first run replace the hand-written
// "## Parameters" section (up to the next H2) with the generated block.
function inject(content, block) {
  const si = content.indexOf(START);
  if (si !== -1) {
    const ei = content.indexOf(END, si);
    if (ei === -1) throw new Error('start marker without end marker');
    return content.slice(0, si) + block + content.slice(ei + END.length);
  }
  const pi = content.indexOf('\n## Parameters');
  if (pi === -1) throw new Error('no marker and no "## Parameters" section to replace');
  const rest = content.slice(pi + 1);
  const next = rest.search(/\n## /);
  const tail = next === -1 ? '' : rest.slice(next + 1);
  return `${content.slice(0, pi + 1)}${block}\n\n${tail}`;
}

let drift = 0;
const seen = new Set();
for (const tool of manifest) {
  const slug = slugOf(tool.name);
  seen.add(`${slug}.mdx`);
  const page = join(toolsDir, `${slug}.mdx`);
  if (!existsSync(page)) {
    console.error(`MISSING PAGE tools/${slug}.mdx for tool ${tool.name}`);
    drift++;
    continue;
  }
  const current = readFileSync(page, 'utf8');
  const updated = inject(current, blockFor(tool));
  if (updated !== current) {
    drift++;
    if (check) console.error(`STALE tools/${slug}.mdx (schema block out of date)`);
    else {
      writeFileSync(page, updated);
      console.log(`wrote tools/${slug}.mdx`);
    }
  }
}
// Orphan pages: a tool page with no live tool behind it.
for (const f of readdirSync(toolsDir)) {
  if (f.endsWith('.mdx') && f !== 'index.mdx' && !seen.has(f)) {
    console.error(`ORPHAN PAGE tools/${f} — no such tool in the manifest`);
    drift++;
  }
}

console.log(`${manifest.length} tools, ${drift} ${check ? 'drifted' : 'updated'} page(s)`);
process.exit(check && drift ? 1 : 0);
