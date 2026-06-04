# ministr-app — DESIGN.md (v4: the AAA consistency anchor)

> **Status:** authored 2026-06-03 from scratch (clean-room). The pre-v4
> DESIGN.md was deleted and is **not** a reference. This document is the
> single source of truth every `aaa-*` phase hard-depends on: each surface
> and primitive is brought to the **AAA bar** defined here, verified against
> the **per-component checklist** (§10) and the **measurable DoD** (§11).
>
> Tokens live in `src/app.css` (`:root` / `.dark` CSS custom properties) and
> the role-class registry `src/lib/ui-tokens.ts`; motion lives in
> `src/lib/motion.ts`. The mechanizable half of the DoD is enforced by
> `scripts/design-lint.cjs` (`npm run design:lint`, wired into `just validate`).
> **Every visual value in a `className` must resolve to a token or a `ui/`
> primitive — never a raw literal.**

---

## §1 — Philosophy & the AAA bar

ministr-app is a **professional developer tool**, not a consumer app. It is a
local-first cockpit over a code-intelligence daemon: dense, fast, keyboard-first,
and legible under sustained use. The v4 bar is not a tweak pass — it is a
**"big difference"** elevation of every surface to the quality of the 2026
reference set (§2).

The AAA bar, in one sentence per axis:

1. **Depth, not decoration.** Hierarchy comes from a disciplined elevation
   model (opaque tiers + a translucent *glass* tier for floating chrome),
   hairline borders, and soft layered shadow — never from arbitrary borders or
   hard offsets.
2. **One accent, used with intent.** Amber is the single accent. Flat fill for
   active state, a sanctioned gradient for *live* emphasis, a soft glow for
   focus/pinned — and nothing else competes with it.
3. **Motion is a first-class citizen, and it is choreographed.** Every state
   change animates through a named motion token (swift / flow / spring). Entry,
   exit, stagger, and layout all have a prescribed motion (§8). All of it is
   `prefers-reduced-motion`-gated.
4. **Bold hierarchy, crisp separation.** (Zed's 2026 lesson: minimal ≠
   low-contrast.) Headings carry real weight; sections are separated by
   deliberate hairlines, not ambiguity.
5. **Accessible by construction.** Token-only color, a visible focus indicator
   that meets WCAG 2.4.13 (AAA), reduced-motion support, and a documented
   solid fallback wherever glass is used (§9).

**Density declaration.** Comfortable-dense. The `html` font-size is the scale
lever (`14px` base, stepping up past 1600/2000/2560/3200/3840px — see
`app.css`). Spacing is rem-based so the whole UI scales as one. Default to the
tighter of two reasonable spacings; never sacrifice legibility for density.

---

## §2 — Reference exemplars (2026)

Captured via 2026-only research. We borrow *patterns*, not pixels.

| Exemplar | What we take |
|----------|--------------|
| **Raycast 2.0 / Apple "Liquid Glass" (macOS Tahoe 26, WWDC25)** | Translucent **layered** material for floating chrome — blurred translucency + specular depth gives "depth and vitality." We adopt a **glass tier** (§4) for the command palette, dialogs, drawers, dropdowns, and toasts. **Caveat we honor:** glass must degrade to a solid surface under *Reduce Transparency* / low-contrast (TidBITS Oct-2025, Apple a11y guidance). |
| **Linear** | Restraint + speed: one accent, generous-but-tight spacing, instant feedback, keyboard-first. Sets our "calm under density" bar. |
| **Zed (2026, MCP-first, perf-first)** | Community's own asks: **stronger accents (bolder weights)** and **clearer separation between UI sections**. Directly informs §1.4 and the heading/separator discipline. Keyboard-driven workflow + ⌘K chord palette is the interaction bar. |
| **Arc** | Choreographed micro-motion and playful-but-purposeful transitions as a quality signal — informs §8. |

---

## §3 — Color system

Warm-dark palette, amber accent. Light + dark are both first-class (`.dark`
overrides in `app.css`). **No raw hex / rgb in `className`** — use the Tailwind
token utilities backed by `@theme` (e.g. `bg-surface`, `text-text-muted`,
`border-border`, `text-accent`).

**Elevation tiers (depth via layering, not just shadow):**

| Token | Role |
|-------|------|
| `--color-bg` | App canvas (warm off-white / warm dark) |
| `--color-surface` | Tier 1 — panels, cards (idle) |
| `--color-surface-raised` | Tier 1.5 — raised cards |
| `--color-surface-overlay` | Tier 2 — headers, hover, selected strip |
| `--color-surface-sunken` | Inset — code blocks, treemap voids |
| `--color-surface-pinned` | Amber-tinted "kept" surface |

**Borders:** `--color-border` (hairline, default), `--color-border-soft`
(faintest divider), `--color-border-hover`.

**Text:** `--color-text` (primary), `--color-text-muted` (secondary prose),
`--color-text-dim` (footnote/caption). Contrast targets in §9.

**Accent & semantics:** `--color-accent` (+`-hover`, `-fg-on`, `-soft`,
`-ring`), `--color-accent-glow` (the glow rgb triplet). Semantic tones:
`--color-success` / `--color-warning` / `--color-danger` / `--color-info`.
Semantic tones carry **meaning** — never use them decoratively.

---

## §4 — Elevation & glass

Two elevation systems, used for different jobs:

**A. Opaque tiers (in-flow content).** Cards, panels, rows, headers stack
through the surface tiers (§3) + the soft shadow scale below. This is the
default for everything that lives *in* the layout.

```
--shadow-xs  resting hairline lift (chips, inputs)
--shadow-sm  cards, raised rows
--shadow-md  popovers, menus, floating cards
--shadow-lg  modal dialogs, the command palette
```

Shadows are **soft and layered** (negative-spread, blurred), never hard
offsets. Use `shadow-xs…shadow-lg` utilities; arbitrary `shadow-[…]` is banned
by design-lint except the one sanctioned `--glow-soft`.

**B. Glass tier (floating chrome only).** *New in v4.* Inspired by Liquid
Glass: floating overlays that sit above content read as **translucent layered
glass** — a blurred backdrop, a translucent surface fill, and a hairline
top-edge highlight for the specular cue. Reserved for: **command palette,
dialogs, drawers, dropdown/menus, toasts.** Never for in-flow content (it costs
legibility where you read the most).

**The scrim.** The dimmed, lightly-blurred backdrop *behind* an overlay is its
own role token — `overlayScrim` in `ui-tokens.ts` (`fixed inset-0 bg-black/50
backdrop-blur-[2px]`). Every modal, drawer, and the command palette compose it
via `cn(overlayScrim, "z-[…] …")` — never hand-rolled. (The `backdrop-blur-[2px]`
literal lives only in the token; design-lint bans it everywhere else, §11.)

The floating panel itself is a **role token** (`glassPanel` in `ui-tokens.ts`),
not ad-hoc classes, so it stays consistent and so the a11y fallback lives in one
place:

- backdrop blur + a translucent `--color-surface` fill (`color-mix` to alpha),
- a hairline `--color-border` ring + a `--color-border-soft` inset top highlight,
- `--shadow-lg` for the lift.

**A11y fallback (mandatory).** Under `prefers-reduced-transparency: reduce`
*or* `prefers-contrast: more`, glass collapses to the **solid** `surface`
+ `shadow-lg`. This lives in `app.css` so every glass user inherits it. Glass
is a finish, never the thing that makes text readable.

---

## §5 — Accent & emphasis

One accent (amber), three sanctioned expressions — nothing else:

1. **Flat fill** (`accentTone` / `bg-accent text-[--color-accent-fg-on]`) —
   the *active/selected* state of a layout-bearing control (nav item, active
   chip, primary button).
2. **Gradient** (`.bg-accent-live` in `app.css`) — **live/animated** emphasis
   only: the active retrieval phase strip, a budget meter under load. The
   shimmering accent→transparent gradient signals "happening now." Never
   static decoration.
3. **Glow** (`--glow-soft` / `.glow-accent`) — a soft halo for *pinned* /
   *just-changed* / *focused-active* elements. The `.ministr-pulse` keyframe is
   the single sanctioned "just changed" cue.

Accent is scarce by design: if everything is accented, nothing is. A surface
should have **one** primary accent moment in view at rest.

---

## §6 — Typography

Two families: **Geist** (sans, UI + prose) and **JetBrains Mono** (mono, data /
labels / code). Scale is a **minor-third (1.2×)** from a 14px base, delivered
fluidly via `clamp()` (`--text-*` in `app.css`).

| Role token (`ui-tokens.ts`) | Use |
|------|------|
| `.text-display` / `headingDisplay` | Page H1 / hero (sans, semibold, tight) |
| `headingChapter` | Section heading in panels/drawers |
| `bodyMuted` | Body prose (secondary contrast) |
| `marginalia` | Footnote / caption tier |
| `labelSmallCap` / `labelMicro` | Mono, uppercase, tracked labels |
| `chapterIndex` | `§N` accent marker prefixing a chapter |

**Rules:** labels are mono-uppercase-tracked with `tracking-[0.06em]` /
`[0.08em]` only (other tracking values are banned by design-lint). Headings are
sans, semibold, tight (negative tracking). Never `font-serif`, never `italic`
for emphasis (use `text-text-dim`). Use the `<H1>` primitive and the heading
tokens — don't hand-roll heading classes.

---

## §7 — Spacing, radius & layout

**Spacing** is the Tailwind rem scale (the `html` font-size lever scales it).
Prefer the multiples already in use (`gap-1.5`, `gap-2`, `px-2.5`, `p-3`…);
keep rhythm consistent within a surface.

**Radius** (role-named, `app.css`): `--radius-button`/`-input` = 8px,
`--radius-card` = 12px, `--radius-data` = 8px, `--radius-pill` = 999px, plus
`--radius-sm/md/lg/xl` = 6/8/12/16. **Never `rounded-none`** (banned) — even
data surfaces are softly rounded.

**Layout (adaptive surfaces):** every top-level surface wraps in
`surfaceContainer` (`@container/surface`). Children adapt with container-query
prefixes (`@min-[600px]/surface:`, `@min-[900px]/surface:`,
`@min-[1200px]/surface:`) — **not** viewport breakpoints. Use `contentNarrow`
(prose/forms), `contentWide` (grids/dashboards), or `contentAdaptive` +
`AdaptiveSurface` (narrow→wide).

---

## §8 — Motion choreography

Motion is mandatory and named. Tokens (mirrored `app.css` ↔ `lib/motion.ts`):

| Token | Duration / ease | Job |
|-------|-----------------|-----|
| **swift** | 140ms · `ease-swift` (standard) | Chrome, hover, nav, toggles — snappy feedback |
| **flow** | 240ms · `ease-flow` (emphasized-decelerate) | Surface/panel transitions, disclosure, drawers |
| **spring** | JS spring (`lib/motion.ts`) | Layout shifts, shared-element (`layoutId`), the nav active pill |

**Choreography prescription** (apply per role; all via `motion.ts` /
`transitionInteractive`, never ad-hoc `duration-[…]`):

- **Hover / press:** swift color/scale via `transitionInteractive`.
- **Surface enter:** `flow` fade-up (small `y` offset → 0).
- **List / grid items:** `flow` enter with a **stagger** (≤ ~40ms step, capped
  total) so groups cascade, not pop.
- **Layout / reorder / active-indicator:** `spring` (the `nav-active` `layoutId`
  pill is the reference implementation).
- **Live/working:** the sanctioned looped cues only — `.ministr-pulse`
  (just-changed), `.bg-accent-live` (active phase), `.ministr-skeleton`
  (loading), `.ministr-blink` (caret). No new infinite animations without a
  token.

**Reduced motion (mandatory).** `app.css` already kills the looped keyframes
and clamps all transitions under `prefers-reduced-motion: reduce`; the
`MotionProvider` in `lib/motion.ts` disables JS springs. Any new animation must
be covered by one of these — verify by toggling the OS setting.

---

## §9 — Accessibility (the measurable floor)

Anchored to WCAG 2.2 (2026 norms):

- **Focus appearance — WCAG 2.4.13 (AAA).** Every interactive element shows a
  visible focus indicator with **≥ 3:1 contrast** against adjacent colors and
  **≥ 2px** thickness (≥ the focused element's perimeter band). The global
  `:focus-visible` rule (`app.css`: 2px accent outline + 2px offset) and the
  `focusRing` token satisfy this. Custom interactive elements that don't get it
  from a primitive **must** apply `focusRing`.
- **Non-text contrast — WCAG 1.4.11.** UI component boundaries, icons, and
  graphical objects (status dots, meters, sparklines) meet **≥ 3:1**.
- **Text contrast — WCAG 1.4.3 (AA ≥ 4.5:1) / 1.4.6 (AAA ≥ 7:1).** `text`,
  `text-muted`, **and `text-dim`** all clear AA on their surfaces in **both
  themes** (light `text-dim` = `#635F58`, dark = `#928D84`); `text-dim` stays
  reserved for non-essential captions but is no longer sub-AA. Status tones, the
  accent, and the Shiki code theme are likewise AA on the surfaces they sit on
  (the inset code surface uses `github-light-high-contrast`).
- **Reduced motion.** Honor `prefers-reduced-motion` (§8) — no exceptions.
- **Reduced transparency / contrast.** Glass (§4) degrades to solid — no
  information conveyed by translucency alone.
- **Forced colors / High Contrast (WHCM).** Under `forced-colors: active` the UA
  strips `background-color` + `box-shadow`, so a custom interactive surface that
  draws its box from bg/shadow alone would vanish. `app.css` ships a
  `@media (forced-colors: active)` floor: a system-colour (`ButtonBorder`) border
  on `button` / `[role="button"|"tab"|"option"|"menuitem"|"switch"|…]` / `summary`
  / `.glass-panel`, plus the focus ring pinned to `Highlight`. The block is scoped
  to the media query, so it changes nothing in the normal themes. **No
  `forced-color-adjust: none` anywhere** — the user's chosen palette always wins
  (Shiki syntax colours go monochrome here, as intended). Verify via Playwright
  `forcedColors: "active"` against `ui/forced-colors.stories.tsx`.
- **Keyboard-first.** Everything actionable is reachable and operable by
  keyboard; the ⌘K command palette is the primary nav accelerator; focus order
  follows reading order; dialogs trap focus and restore it on close.
- **Semantics.** Real roles/labels (`aria-*`) on custom controls; icon-only
  buttons carry an accessible name. No nested interactive controls — a row that
  both "inspects" and "opens" uses two sibling buttons, never a button inside a
  `role="button"` row.

### The floor is mechanical, not manual

`@storybook/addon-a11y` runs **axe on every story** as part of the gate, via
`@storybook/addon-vitest` (stories become Vitest component tests in Playwright
Chromium). `a11y.test: "error"` (`.storybook/preview.tsx`) makes any WCAG
violation **fail `pnpm test`**. The `storybook` (light) and `storybook-dark`
Vitest projects render every story in **both themes**, so contrast is checked on
both surface tiers; animations are forced to their final frame so axe never
snapshots text mid-fade. The M·F·R·S checklist below is now pass/fail in CI, not
a manual review — a regression that drops any text/icon below its WCAG floor
turns the gate red. (Verified: a seeded low-contrast story fails the run.)

---

## §10 — Per-component AAA checklist

The unit of work for every `aaa-*` phase. Each row is scored on six axes:

- **T** — token-purity (no raw literals; values resolve to tokens / primitives)
- **E** — elevation/glass correct for its role (§4)
- **M** — motion choreographed (§8) incl. reduced-motion
- **F** — focus-visible on every interactive element (§9)
- **R** — reduced-motion / reduced-transparency / forced-colors honored
- **S** — state coverage (idle · hover · active · empty · loading · error ·
  disabled — those that apply)

Mark each `✓` (meets v4) / `~` (partial) / `—` (n/a). A component is **AAA-done**
when every applicable axis is `✓` and it passes design-lint.

### Primitives — `src/components/ui/` (30 files)

| # | Primitive | T | E | M | F | R | S |
|---|-----------|---|---|---|---|---|---|
| 1 | button | | | | | | |
| 2 | card | | | | | | |
| 3 | badge | | | | | | |
| 4 | progress | | | | | | |
| 5 | status-dot | | | | | | |
| 6 | empty-state | | | | | | |
| 7 | error-callout | | | | | | |
| 8 | content-tray | | | | | | |
| 9 | disclosure | | | | | | |
| 10 | toggle (+ ToggleRow) | | | | | | |
| 11 | metric-tile | | | | | | |
| 12 | vital-card | | | | | | |
| 13 | budget-bar | | | | | | |
| 14 | budget-ring | | | | | | |
| 15 | token-economics-bar | | | | | | |
| 16 | sparkline | | | | | | |
| 17 | number-ticker | | | | | | |
| 18 | activity-feed | | | | | | |
| 19 | coherence-feed | | | | | | |
| 20 | chip-group | | | | | | |
| 21 | filter-pill | | | | | | |
| 22 | corpus-select | | | | | | |
| 23 | labeled-card | | | | | | |
| 24 | labeled-row | | | | | | |
| 25 | confirm-dialog | | | | | | |
| 26 | surface-sidebar | | | | | | |
| 27 | adaptive-surface | | | | | | |
| 28 | heading (H1) | | | | | | |
| 29 | turn-block | | | | | | |
| 30 | brutal-icons | | | | | | |

### Surfaces — `src/components/surfaces/`

| Surface | Phase | T | E | M | F | R | S |
|---------|-------|---|---|---|---|---|---|
| Projects (+ LinkedProjectsPanel, ProjectSessions) | `aaa-projects` | | | | | | |
| Sessions | `aaa-sessions` | | | | | | |
| Ask (AskSurface/Answer/Citation/Empty/Input/Status/PinnedAnswers) | `aaa-ask` | | | | | | |
| Cloud (CloudPanel: 6 sections + dialogs) | `aaa-cloud` | | | | | | |
| Settings (General/AiAssistants/About/Server + settings-primitives) | `aaa-settings` | | | | | | |
| Explore (ExploreSurface) | `f-explore-facelift` | | | | | | |
| Onboarding (`components/Onboarding.tsx` + cloud OnboardingWizard) | `aaa-onboarding` | | | | | | |

### Chrome — `src/components/chrome/`

| Chrome | Phase | T | E | M | F | R | S |
|--------|-------|---|---|---|---|---|---|
| TopBar | `aaa-chrome` | | | | | | |
| ProjectPicker (pill) | `aaa-chrome` | | | | | | |
| Sidebar (nav rail) | `aaa-chrome` | | | | | | |
| CommandPalette (⌘K) | `aaa-chrome` | | | | | | |
| Toasts / DaemonDot | `aaa-chrome` | | | | | | |

---

## §11 — Measurable Definition of Done

A phase is **done** only when **both** halves pass.

### A. Mechanized (enforced by `design:lint`, gates `just validate`)

`scripts/design-lint.cjs` runs against comment-stripped source and fails CI on
any violation. The rules, **re-derived from this spec**:

| Rule | Spec § | Rationale |
|------|--------|-----------|
| No banned pre-v4 literals (`tracking-[0.05em]`/`[0.1em]`, `transition-none`, `rounded-none`, `border-2`, `font-serif`, `italic`, dead `ministr-*` classes) | §1, §6, §7, §8 | Consistency floor — keep the inherited denylist |
| No arbitrary `shadow-[…]` except `--glow-soft` | §4 | Elevation goes through the shadow scale |
| **No raw color in `className`** — no `#hex`, `rgb(`/`rgba(`/`hsl(` arbitrary color, no `text-[#…]`/`bg-[#…]` | §3 | Token-only color (the contrast guarantees live in the tokens) |
| **No arbitrary `backdrop-blur-[…]` / `bg-[…]/NN` alpha** outside the glass token | §4 | Glass is a single role token with the a11y fallback in one place |
| **No arbitrary `duration-[…]`** outside `lib/motion.ts` | §8 | Motion goes through the swift/flow/spring tokens |

(Tokens/contract files — `lib/ui-tokens.ts`, `lib/motion.ts`, `main.tsx` — are
allow-listed so they can *define* the sanctioned values.)

Plus the existing gates: `tsc --noEmit`, `vite build`.

### B. Manual rubric (per surface/primitive, recorded in the phase)

Score each checklist axis (§10) and compute a **consistency score (0–100)**:
`100 × (✓ axes) / (applicable axes)`. A phase ships at **100** (every applicable
axis ✓) — anything less is filed as a follow-up, never silently dropped.

Plus, per phase:

- **Before/after** capture (screenshot or described delta) of the surface.
- **Reduced-motion pass:** toggle the OS setting, confirm no motion regressions.
- **Keyboard pass:** tab through every interactive element; focus visible
  throughout; ⌘K reachable.
- `design:lint` + `tsc` + `vite build` green; **committed on `main`**.

---

## §12 — How to apply this spec (per `aaa-*` phase)

1. Open the phase; pull this doc's §10 checklist for the target surface/group.
2. For each component: audit against the six axes; replace any raw literal with
   a token or `ui/` primitive; add the glass token where it's floating chrome;
   wire motion through `motion.ts`; confirm `focusRing` + reduced-motion.
3. Run `npm run design:lint && npm run build` (or `just validate`) — green.
4. Capture before/after; record the consistency score; commit on `main`.
5. Tick the checklist rows. The anchor doc is the shared scoreboard.
