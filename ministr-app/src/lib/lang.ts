/**
 * Map a file path / name to a Shiki language id for syntax highlighting.
 *
 * Frontend mirror of the daemon's `lang_from_path` (commands.rs) — kept here
 * so any UI that shows a code excerpt can infer the grammar from a filename
 * without a round-trip. Unknown extensions fall back to `"text"` (Shiki
 * renders them as plain, un-highlighted code).
 */

const EXT_TO_LANG: Record<string, string> = {
  rs: "rust",
  ts: "typescript",
  tsx: "tsx",
  js: "javascript",
  jsx: "jsx",
  mjs: "javascript",
  cjs: "javascript",
  py: "python",
  pyi: "python",
  go: "go",
  java: "java",
  kt: "kotlin",
  kts: "kotlin",
  swift: "swift",
  c: "c",
  h: "c",
  cc: "cpp",
  cpp: "cpp",
  cxx: "cpp",
  hpp: "cpp",
  hh: "cpp",
  cs: "csharp",
  rb: "ruby",
  php: "php",
  scala: "scala",
  dart: "dart",
  ex: "elixir",
  exs: "elixir",
  erl: "erlang",
  zig: "zig",
  lua: "lua",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  fish: "fish",
  ps1: "powershell",
  sql: "sql",
  json: "json",
  jsonc: "jsonc",
  toml: "toml",
  yaml: "yaml",
  yml: "yaml",
  xml: "xml",
  html: "html",
  htm: "html",
  css: "css",
  scss: "scss",
  sass: "sass",
  less: "less",
  md: "markdown",
  mdx: "mdx",
  markdown: "markdown",
  proto: "proto",
  graphql: "graphql",
  gql: "graphql",
  dockerfile: "docker",
  makefile: "makefile",
  vue: "vue",
  svelte: "svelte",
};

const SPECIAL_NAMES: Record<string, string> = {
  dockerfile: "docker",
  makefile: "makefile",
  "cargo.lock": "toml",
  ".gitignore": "text",
};

/** Resolve a filename or path to a Shiki language id. */
export function langFromPath(pathOrName: string | null | undefined): string {
  if (!pathOrName) return "text";
  const base = pathOrName.split(/[/\\]/).pop()?.toLowerCase() ?? "";
  if (SPECIAL_NAMES[base]) return SPECIAL_NAMES[base];
  const dot = base.lastIndexOf(".");
  if (dot < 0) return "text";
  const ext = base.slice(dot + 1);
  return EXT_TO_LANG[ext] ?? "text";
}
