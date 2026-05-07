/**
 * Central registry of class-string tokens for the visual design language.
 *
 * Every visual decision in the app should resolve through one of the
 * tokens below or a primitive in `components/ui/*`. Five role mappings
 * are codified here — adding a new style should pick from these or
 * extend the registry, not inline a one-off.
 *
 * ## Role → text tier
 * - `headingDisplay`   — page H1 (Plex Serif 2xl, normal weight).
 * - `headingChapter`   — entity §N / zone heading (Plex Serif lg bold).
 * - body primary       — `text-text` (default; workhorse).
 * - `bodyMuted`        — secondary prose, descriptions, hints.
 * - `marginalia`       — Plex Serif italic dim; footnote tier.
 * - `labelSmallCap`    — mono uppercase tracked label (~12px).
 * - `labelMicro`       — mono uppercase tracked label (~11px); used for
 *   stat-cell labels, chip captions, table header cells.
 *
 * ## Role → surface tier
 * - `surfacePanel`        — every Card / Zone idle.
 * - `surfacePanelActive`  — hover, selected, header strips.
 * - `surfacePanelSunken`  — code blocks, treemap voids.
 * - The page bg uses `bg-bg`; cards needing extra contrast vs page can
 *   use `bg-surface-raised` directly (kept for compat, but prefer
 *   `surfacePanel` everywhere new).
 *
 * ## Role → border weight (the brutalist signature lives here)
 * - `separatorThin`       — list-row sub-divider.
 * - `separatorBold`       — zone header underline, table row separator.
 * - `containerDefault`    — idle Card / Zone outer.
 * - `containerActive`     — selected / highlighted outer (accent edge).
 *
 * ## Role → radius
 * Use Tailwind utilities: `rounded-none` (data: tables, code, treemap)
 * or `rounded-sm` (controls: buttons, inputs, chips, pills, kbd). Both
 * map through `--radius-*` tokens in App.css. Do not use inline
 * `style={{ borderRadius: ... }}`.
 *
 * ## Role → shadow elevation
 * Use Tailwind: `shadow-xs|sm|md|lg`. They compose from `--shadow-*`
 * tokens (hard offset, no blur). Do not write `shadow-[Npx_Npx_…]`.
 *
 * ---
 *
 * Brutalist signature: every label is mono uppercase tracking-[0.05em];
 * the icon box is a 2px-bordered square; the accent-tone fragment is
 * solid accent. No transitions on hover. Hard-step animations only.
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
  "text-mono-mini font-mono uppercase tracking-[0.05em] text-text-dim";

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

/* ---- Surface tier (Phase 2+) --------------------------------------- */

/** Every Card / Zone idle background. */
export const surfacePanel = "bg-surface";

/** Hover, selected, header strip background. */
export const surfacePanelActive = "bg-surface-overlay";

/** Code blocks, treemap voids — visually inset from the page. */
export const surfacePanelSunken = "bg-surface-sunken";

/* ---- Border weight (brutalist signature) --------------------------- */

/** List-row sub-divider — single hairline. */
export const separatorThin = "border-b border-border-soft";

/** Zone header underline, table row separator — bold 2px stroke. */
export const separatorBold = "border-b-2 border-border";

/** Idle Card / Zone outer. */
export const containerDefault = "border border-border-soft";

/** Selected / highlighted outer — accent edge, doubled width. */
export const containerActive = "border-2 border-accent";
