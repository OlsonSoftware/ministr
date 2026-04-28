/**
 * Tailwind class-string constants for repeated patterns across the UI.
 *
 * Pulled out so callers can `cn(labelSmallCap, ...)` instead of inlining
 * the same long class string in five places. Not a design-system layer —
 * just shared text. Keep this module dependency-free.
 */

/** Small-caps section label (~11px, uppercase, tracked). Used as the
 *  header inside compact data panels (Overview side panels, ProjectDetail
 *  sections, vital-card titles). */
export const labelSmallCap =
  "text-[11px] font-semibold uppercase tracking-wider text-text-dim";

/** Accent-tinted square icon container. Caller still picks the size via
 *  `cn(iconBox, "h-8 w-8")`. */
export const iconBox =
  "grid place-items-center rounded-lg bg-[var(--color-accent-soft)] text-accent";

/** Tinted "active state" tone fragment — the `bg-accent-soft + text-accent`
 *  half of `iconBox`, without the layout. Use inside ternaries on
 *  layout-bearing buttons (rail items, filter pills, theme pickers) where
 *  you want only the color tone, not a full tinted box. */
export const accentTone =
  "bg-[var(--color-accent-soft)] text-accent";

/** Even-smaller-caps data label (~10px, no `font-semibold`). Used inline
 *  inside compact stat cells where `labelSmallCap`'s 11px + semibold reads
 *  as too heavy. Sibling of `labelSmallCap`, not a replacement. */
export const labelMicro =
  "text-[10px] uppercase tracking-wider text-text-dim";
