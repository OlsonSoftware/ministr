/**
 * Central registry of class-string tokens for the "Cockpit" design
 * language. Every visual decision should resolve through one of these
 * or a primitive in `components/ui/*`.
 *
 * Export names are unchanged from the previous (brutalist) registry so
 * call sites keep compiling; the *semantics* are rebuilt:
 *
 * - Headings: tight sans display scale (no serif).
 * - Labels: mono, uppercase, gently tracked — used sparingly for
 *   stat captions / table headers, never for buttons or prose.
 * - Surfaces: layered elevation tiers.
 * - Borders: hairline by default; accent ring for active.
 * - Radius: soft (use Tailwind `rounded-md|lg`); shadow for elevation.
 */

/* ---- Label tier (mono, uppercase, tracked) ---- */

/** Section label (~12px). Header inside compact data panels. */
export const labelSmallCap =
  "text-xs font-mono font-medium uppercase tracking-[0.08em] text-text-dim";

/** Smallest label (~11px). Stat-cell captions, table headers, chips. */
export const labelMicro =
  "text-mono-mini font-mono uppercase tracking-[0.08em] text-text-dim";

/** Square icon container — soft rounded, hairline border. */
export const iconBox =
  "grid place-items-center border border-border bg-surface-overlay text-text rounded-md";

/** Solid-accent "active" tone fragment for layout-bearing toggles. */
export const accentTone = "bg-accent text-[var(--color-accent-fg-on)]";

/* ---- Heading tier (sans, tight) ---- */

/** Page H1 — display sans, semibold, tight tracking. */
export const headingDisplay =
  "font-sans text-2xl font-semibold tracking-[-0.01em] text-text leading-tight";

/** Chapter / section heading inside panels & drawers. */
export const headingChapter =
  "font-sans text-base font-semibold tracking-[-0.005em] text-text leading-snug";

/** Body prose — secondary contrast tier (workhorse for hints/desc). */
export const bodyMuted = "font-sans text-sm text-text-muted leading-relaxed";

/** Marginalia — faint footnote tier (no longer italic serif). */
export const marginalia = "font-sans text-xs text-text-dim leading-snug";

/** Section index — `§N` marker prefixing a chapter heading. */
export const chapterIndex =
  "font-mono text-xs font-medium text-accent tabular-nums shrink-0";

/* ---- Surface tier ---- */

/** Every Card / Zone idle background (tier 1). */
export const surfacePanel = "bg-surface";

/** Hover, selected, header strip (tier 2). */
export const surfacePanelActive = "bg-surface-overlay";

/** Code blocks, treemap voids — inset (sunken). */
export const surfacePanelSunken = "bg-surface-sunken";

/* ---- Border / elevation ---- */

/** List-row sub-divider — faintest hairline. */
export const separatorThin = "border-b border-border-soft";

/** Zone header underline, table row separator — hairline. */
export const separatorBold = "border-b border-border";

/** Idle Card / Zone outer — hairline + soft radius. */
export const containerDefault = "border border-border rounded-lg";

/** Selected / highlighted outer — accent ring. */
export const containerActive =
  "border border-accent rounded-lg shadow-[var(--glow-soft)]";
