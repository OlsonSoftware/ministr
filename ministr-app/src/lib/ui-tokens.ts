/**
 * Role tokens — the class-string registry for the ministr design system.
 * Every visual decision resolves through one of these or a primitive in
 * `components/ui/*`. See DESIGN.md for the full design contract.
 *
 * Token tiers:
 * - Labels: mono, uppercase, tracked (§ Typography scale step −1/−2)
 * - Headings: sans, semibold, tight (§ Typography scale step +1/+3)
 * - Surfaces: layered elevation (§ Color system depth model)
 * - Borders: hairline by default; accent ring for active
 * - Interaction: transitionInteractive, focusRing (§ Accessibility)
 * - Layout: surfaceContainer, content width tokens (§ Layout)
 */

/* ---- Label tier (mono, uppercase, tracked) ----
   Scale: minor-third 1.2× from 14px base.
   See DESIGN.md "Typography scale" for the full derivation. */

/** Scale −1 (micro, ~12px). Section label in compact data panels. */
export const labelSmallCap =
  "text-xs font-mono font-medium uppercase tracking-[0.08em] text-text-dim";

/** Scale −2 (nano, ~11px). Stat-cell captions, table headers, chips. */
export const labelMicro =
  "text-mono-mini font-mono uppercase tracking-[0.08em] text-text-dim";

/** Square icon container — soft rounded, hairline border. */
export const iconBox =
  "grid place-items-center border border-border bg-surface-overlay text-text rounded-md";

/** Solid-accent "active" tone fragment for layout-bearing toggles. */
export const accentTone = "bg-accent text-[var(--color-accent-fg-on)]";

/* ---- Heading tier (sans, tight) ----
   Scale: minor-third 1.2× from 14px base. */

/** Scale +3 (display, ~24px). Page H1 — sans, semibold, tight. */
export const headingDisplay =
  "font-sans text-2xl font-semibold tracking-[-0.01em] text-text leading-tight";

/** Scale +1 (chapter, ~16px). Section heading inside panels & drawers. */
export const headingChapter =
  "font-sans text-base font-semibold tracking-[-0.005em] text-text leading-snug";

/** Scale 0 (base, 14px). Body prose — secondary contrast. */
export const bodyMuted = "font-sans text-sm text-text-muted leading-relaxed";

/** Scale −1 (micro, ~12px). Faint footnote tier. */
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

/* ---- Interaction & motion ---- */

/** The one sanctioned hover/active transition for interactive elements.
 *  Clickable things must animate their state change — use this, never
 *  the disabled-transition utility. */
export const transitionInteractive =
  "transition-colors duration-150 ease-out";

/** Focus ring for custom interactive elements (buttons/rows/inputs that
 *  don't get it from a primitive). */
export const focusRing =
  "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent";

/* ---- Dividers (alias the separators with intent-named exports) ---- */

/** Between list rows — faintest hairline. */
export const dividerRow = separatorThin;

/** Section header underline / table separator — hairline. */
export const dividerSection = separatorBold;

/* ---- Chip (must match the <Badge> pill shape) ---- */

/** Idle filter/selector chip — rounded-full pill, hairline. */
export const chip =
  "inline-flex items-center gap-1.5 rounded-full border border-border " +
  "bg-surface px-2.5 py-0.5 font-mono text-mono-mini font-medium " +
  "uppercase tracking-[0.06em] text-text-muted " +
  "hover:text-text hover:border-border-hover hover:bg-surface-overlay " +
  transitionInteractive +
  " cursor-pointer";

/** Selected chip — solid accent. */
export const chipActive =
  "inline-flex items-center gap-1.5 rounded-full border border-accent " +
  "bg-accent px-2.5 py-0.5 font-mono text-mono-mini font-medium " +
  "uppercase tracking-[0.06em] text-[var(--color-accent-fg-on)] " +
  "cursor-pointer";

/* ---- Layout tier (adaptive surface system) ---- */

/** Container-query wrapper class for top-level surfaces.
 *  Apply to the outermost div of each surface so children can use
 *  @min-[600px]/surface:, @min-[900px]/surface:, @min-[1200px]/surface: */
export const surfaceContainer = "@container/surface h-full min-h-0";

/** Narrow content constraint — for prose-heavy areas that shouldn't
 *  expand beyond comfortable reading width (forms, about panels). */
export const contentNarrow = "max-w-3xl mx-auto";

/** Wide content — no max-width; fills available container space.
 *  For grids, master-detail layouts, dashboards. */
export const contentWide = "w-full";

/** Adaptive content — narrow below @md, wide above. Use inside an
 *  AdaptiveSurface wrapper. Children still need to apply their own
 *  responsive grid/flex classes using container-query prefixes. */
export const contentAdaptive =
  "w-full max-w-3xl @min-[900px]/surface:max-w-none mx-auto @min-[900px]/surface:mx-0";

/* ---- Overlay tier (DESIGN.md §4 — floating chrome) ---- */

/** Modal/overlay scrim — the dimmed, lightly-blurred backdrop behind a
 *  dialog, drawer, or the command palette. The single sanctioned overlay
 *  backdrop (the `backdrop-blur-[2px]` value lives here so call sites never
 *  hand-roll it). Compose with `cn(overlayScrim, "z-[…] …")` for layering. */
export const overlayScrim = "fixed inset-0 bg-black/50 backdrop-blur-[2px]";

/** Glass panel — the translucent layered material for FLOATING chrome only
 *  (command palette, dialogs, drawers, dropdowns, toasts), per DESIGN.md §4.
 *  Backed by the `.glass-panel` CSS utility (app.css), which carries the
 *  blur + specular highlight AND the mandatory reduced-transparency solid
 *  fallback. Never use on in-flow content. Compose with `cn(glassPanel, …)`. */
export const glassPanel = "glass-panel";
