import {
  siCplusplus,
  siElixir,
  siGo,
  siJavascript,
  siKotlin,
  siOpenjdk,
  siPhp,
  siPython,
  siRuby,
  siRust,
  siScala,
  siSwift,
  siTypescript,
} from "simple-icons";

/**
 * Tech-icon registry for the per-project stack row (gui-card-tech-icons).
 *
 * Logo path data + the official brand hex come from the licensed
 * `simple-icons` package — we never transcribe a brand mark by hand. Only
 * the ~14 detected languages are imported, so the bundle stays tiny
 * (`sideEffects: false` lets the bundler drop the rest).
 *
 * Two slugs have no clean licensed mark: Java's coffee-cup is trademark-
 * removed (we use the OpenJDK mark, labelled "Java"), and there is no C#
 * icon at all — that one falls back to a neutral lettermark (`path`/`hex`
 * omitted).
 */
export interface TechEntry {
  /** User-facing label (also the accessible name). */
  title: string;
  /** simple-icons 24×24 path, when a licensed mark exists. */
  path?: string;
  /** Official brand hex (no leading `#`) — the hover/focus colour. */
  hex?: string;
  /** Short lettermark for techs with no licensed icon (e.g. "C#"). */
  mark?: string;
}

const TECH: Record<string, TechEntry> = {
  rust: { title: "Rust", path: siRust.path, hex: siRust.hex },
  typescript: { title: "TypeScript", path: siTypescript.path, hex: siTypescript.hex },
  javascript: { title: "JavaScript", path: siJavascript.path, hex: siJavascript.hex },
  python: { title: "Python", path: siPython.path, hex: siPython.hex },
  go: { title: "Go", path: siGo.path, hex: siGo.hex },
  java: { title: "Java", path: siOpenjdk.path, hex: siOpenjdk.hex },
  kotlin: { title: "Kotlin", path: siKotlin.path, hex: siKotlin.hex },
  php: { title: "PHP", path: siPhp.path, hex: siPhp.hex },
  ruby: { title: "Ruby", path: siRuby.path, hex: siRuby.hex },
  csharp: { title: "C#", mark: "C#" },
  swift: { title: "Swift", path: siSwift.path, hex: siSwift.hex },
  scala: { title: "Scala", path: siScala.path, hex: siScala.hex },
  cpp: { title: "C++", path: siCplusplus.path, hex: siCplusplus.hex },
  elixir: { title: "Elixir", path: siElixir.path, hex: siElixir.hex },
};

/** The registry entry for a detected slug, or `undefined` if unknown. */
export function techEntry(slug: string): TechEntry | undefined {
  return TECH[slug.toLowerCase()];
}
