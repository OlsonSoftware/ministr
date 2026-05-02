/**
 * Tailwind class-string constants for repeated patterns across the UI.
 *
 * Pulled out so callers can `cn(labelSmallCap, ...)` instead of inlining
 * the same long class string in five places. Not a design-system layer —
 * just shared text. Keep this module dependency-free.
 *
 * Brutalist update: every label is mono uppercase tracking-[0.05em]; the icon
 * box is a 2px-bordered square; the accent-tone fragment is solid accent.
 */

/** Section label (~12px, uppercase mono, semibold, lightly tracked).
 *  Used as the header inside compact data panels. Tracking dropped to 0.05em
 *  for legibility — caps already provide visual weight; extra spacing slows
 *  the read. */
export const labelSmallCap =
  "text-xs font-mono font-semibold uppercase tracking-[0.05em] text-text-dim";

/** Even-smaller label (~11px, uppercase mono, no semibold). Used inline
 *  inside compact stat cells where `labelSmallCap` reads as too heavy. */
export const labelMicro =
  "text-[0.6875rem] font-mono uppercase tracking-[0.05em] text-text-dim";

/** Square 2px-bordered icon container. Caller picks the size via
 *  `cn(iconBox, "h-8 w-8")`. */
export const iconBox =
  "grid place-items-center border-2 border-border bg-surface text-text";

/** Solid-accent "active state" tone fragment. Use inside ternaries on
 *  layout-bearing buttons (rail items, filter pills, theme pickers) where
 *  you want only the color tone, not a full bordered box. */
export const accentTone =
  "bg-accent text-[var(--color-accent-fg-on)]";

/* ---- Field Manual roles (Phase 1+) ---------------------------------- */

/** Page-title display — Plex Serif sentence-case. Rendered at the top of
 *  each major view (Search, Symbols, Bridge, Sessions, Logs, Settings,
 *  Projects). Replaces the old MONO UPPERCASE TRACKED page anchors. */
export const headingDisplay =
  "font-serif text-2xl font-normal text-text leading-tight";

/** Chapter heading — Plex Serif used for major section titles inside
 *  EntityPanel views, Settings groups, Onboarding step pages, etc.
 *  Pairs naturally with a `§N` index marker rendered alongside. */
export const headingChapter =
  "font-serif text-lg font-bold text-text leading-snug";

/** Body prose — Plex Sans sentence-case at the secondary contrast tier.
 *  Use for descriptions, hints, list-row body text. The dim tier is
 *  reserved for marginalia / footnotes; this tier is the workhorse. */
export const bodyMuted = "font-sans text-sm text-text-muted leading-normal";

/** Marginalia — faint italic Plex Serif. Renders as a left/right-margin
 *  note rather than a floating tooltip. Knuth-style technical doc. */
export const marginalia =
  "font-serif text-xs italic text-text-dim leading-snug";

/** Section index — `§N` marker that prefixes a chapter heading. Rendered
 *  in Plex Serif at chapter-size, with tabular-numeric digits so multiple
 *  `§1 / §2 / §10` align in a column. */
export const chapterIndex =
  "font-serif text-lg font-normal text-text-dim tabular-nums shrink-0";
