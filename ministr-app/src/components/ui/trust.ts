/**
 * The trust vocabulary (DESIGN.md §2.1, §7) — the single definition of
 * the four states every surface speaks. Glyphs are distinct SHAPES so the
 * state never relies on color alone (WCAG 1.4.1); tone classes are the
 * only sanctioned use of the trust colors on text-sized marks.
 */
export type TrustState = "ok" | "stale" | "hidden" | "updating";

export interface TrustMeta {
  /** Distinct shape — the state's letterform. */
  glyph: string;
  /** The plain word screen readers and labels use. */
  word: string;
  /** Tone utility for the MARK only — never sentence text (§2.4). */
  tone: string;
}

export const TRUST: Record<TrustState, TrustMeta> = {
  ok: { glyph: "✓", word: "up to date", tone: "text-ok" },
  stale: { glyph: "⚠", word: "behind your changes", tone: "text-stale" },
  hidden: { glyph: "✗", word: "hidden from your AI", tone: "text-hidden" },
  // "⟳" not "◌": the thin circle disappears at row sizes (scrutiny finding).
  updating: { glyph: "⟳", word: "updating", tone: "text-brand" },
};
