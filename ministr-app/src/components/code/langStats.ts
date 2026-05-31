/**
 * Pure language-breakdown derivation for the Code landing overview.
 *
 * The corpus file list carries no language field, so we infer it from each
 * file's extension and aggregate counts. This is display-only (a proportion
 * bar + labels), so unknown extensions collapse into "other" rather than
 * inventing a language.
 */

/** Map a path's extension to a human language label, or null for "other". */
function labelForPath(path: string): string | null {
  const dot = path.lastIndexOf(".");
  const slash = path.lastIndexOf("/");
  if (dot < 0 || dot < slash) return null;
  const ext = path.slice(dot + 1).toLowerCase();
  const map: Record<string, string> = {
    rs: "Rust",
    ts: "TypeScript",
    tsx: "TypeScript",
    js: "JavaScript",
    jsx: "JavaScript",
    mjs: "JavaScript",
    cjs: "JavaScript",
    py: "Python",
    go: "Go",
    java: "Java",
    rb: "Ruby",
    c: "C",
    h: "C",
    cc: "C++",
    cpp: "C++",
    cxx: "C++",
    hpp: "C++",
    cs: "C#",
    php: "PHP",
    swift: "Swift",
    kt: "Kotlin",
    scala: "Scala",
    sh: "Shell",
    bash: "Shell",
    zsh: "Shell",
    lua: "Lua",
    dart: "Dart",
    ex: "Elixir",
    exs: "Elixir",
    hs: "Haskell",
    json: "JSON",
    toml: "TOML",
    yaml: "YAML",
    yml: "YAML",
    md: "Markdown",
    markdown: "Markdown",
    html: "HTML",
    css: "CSS",
    scss: "CSS",
    sql: "SQL",
  };
  return map[ext] ?? null;
}

export interface LangStat {
  /** Language label, or "Other". */
  label: string;
  /** File count for this language. */
  count: number;
  /** Share of the total file count in [0, 1]. */
  fraction: number;
}

/**
 * Aggregate files by inferred language, largest first, capped to `top`
 * entries with the remainder folded into a trailing "Other" bucket.
 */
export function langStats(paths: string[], top = 6): LangStat[] {
  if (paths.length === 0) return [];
  const counts = new Map<string, number>();
  for (const p of paths) {
    const label = labelForPath(p) ?? "Other";
    counts.set(label, (counts.get(label) ?? 0) + 1);
  }
  const total = paths.length;

  // "Other" always sorts last regardless of size; real langs sort by count.
  const entries = [...counts.entries()].sort((a, b) => {
    if (a[0] === "Other") return 1;
    if (b[0] === "Other") return -1;
    return b[1] - a[1];
  });

  const head = entries.slice(0, top);
  const tailCount = entries.slice(top).reduce((s, [, c]) => s + c, 0);
  const merged = [...head];
  if (tailCount > 0) {
    const existingOther = merged.find((e) => e[0] === "Other");
    if (existingOther) existingOther[1] += tailCount;
    else merged.push(["Other", tailCount]);
  }

  return merged.map(([label, count]) => ({
    label,
    count,
    fraction: count / total,
  }));
}
