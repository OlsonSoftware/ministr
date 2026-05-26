# ministr-app — Design contract (v3)

This is the **single source of truth** for UI consistency. Every component
must obey it; `pnpm design:lint` enforces the hard rules in CI.

## Design philosophy

### Product class

ministr is a **professional developer tool**. Its users stare at it for
hours while navigating code intelligence results, managing corpora, and
reading structured data. Every design decision flows from this: the UI
optimizes for *sustained-use information work*, not first-impression wow.

### Density: dense with hierarchy

High information density is a feature, not a bug. Whitespace serves
structure (grouping related items, separating unrelated sections) — never
decoration. Every pixel earns its place.

The governing principle: **reduce cycle load**. The fewer interactions
(scrolls, clicks, eye movements) between the user's question and their
answer, the better. Density achieves this; hierarchy makes it scannable.

Reference implementations: Linear (density + keyboard-first), Warp
(developer-tool typography), Arc (spatial motion), Raycast (speed + minimal
chrome). What we take from each:

| Reference | We adopt | We reject |
|---|---|---|
| Linear | Dense tables, keyboard shortcuts everywhere, no wasted rows | Their aggressive whitespace in settings panels |
| Warp | Monospace-dominant data display, dark-first palette | Their custom renderer (we use standard web tech) |
| Arc | Purposeful spring animations for spatial transitions | Their creative tab UI (too novel for a tool) |
| Raycast | Instant feedback, minimal chrome, command palette | Their single-surface constraint (we have 6 surfaces) |

### Dark-first

Dark mode is the design target. Developers work in dark terminals, dark
IDEs, dark everything. A bright tool window at 2am is hostile. All color
decisions are made on the dark palette first; light mode is a mechanical
inversion, not a co-equal design surface.

### Motion: communicate, don't decorate

Every animation must answer one of these questions:
1. **Where did this come from?** (spatial origin — spring/layout animations)
2. **Where is this going?** (spatial destination — exit animations)
3. **What just changed?** (content transition — fade/slide)
4. **I did something, was it received?** (acknowledgement — swift flash)

An animation that answers none of these is decoration. Remove it.
_(Apple HIG: "Use motion to provide context, share feedback, and
communicate relationships between elements.")_

### Typography: mono for data, sans for prose

Monospace is the primary typeface. Symbol names, file paths, counts,
identifiers, status labels, timestamps — all mono. The user lives in a
monospace world; the tool should feel native to it.

Sans-serif is reserved for:
- Surface headings (H1, section titles)
- Multi-sentence descriptions and empty-state prose
- Onboarding/marketing-adjacent text

Never mix: a label that's half monospace and half sans is a design bug.

### NOT this (anti-patterns with rationale)

| Don't | Why |
|---|---|
| Gratuitous hover effects (scale, glow, parallax) | Developer tools are keyboards-first; hover is secondary feedback, not a reward |
| Gradient borders / glass-morphism / blur-heavy overlays | These signal "consumer app"; ministr is infrastructure |
| Creative loading animations (bouncing dots, pulsing rings) | A spinner or skeleton is honest; animation that entertains while you wait implies the wait is acceptable |
| Illustrations / mascots / emoji in the chrome | These reduce information density and signal "not serious" |
| Color as sole differentiator | Always pair with shape/icon — ~8% of male users are color-blind |
| Italic for emphasis | Dim (`text-text-dim`) communicates hierarchy without the readability cost of italic in UI text |
| Card-within-card nesting | One level of containment maximum; nesting adds visual noise without information |

## Typography scale

### Ratio: 1.2 (minor third)

Base size: **14px** (`html { font-size: 14px }`, scaling to 15–22px on
wide displays via resolution breakpoints). The minor third (1.2×) ratio
provides tight steps suited to high-density UI — major third (1.25) or
perfect fourth (1.333) would waste vertical space between hierarchy levels.

| Step | Size | Tailwind | Role | Line-height |
|---|---|---|---|---|
| −2 (nano) | ~10px | `text-mono-micro` | Nav rail labels, histogram axis | `leading-none` (1.0) |
| −1 (micro) | ~11px | `text-mono-mini` | Stat captions, chip labels, table headers | `leading-tight` (1.25) |
| 0 (base) | 14px | `text-sm` / default | Body text, descriptions, list items | `leading-relaxed` (1.6) |
| +1 (chapter) | ~16px | `text-base` | Section headings, panel titles | `leading-snug` (1.375) |
| +2 (title) | ~20px | `text-xl` | Surface-level H1 (rare) | `leading-tight` (1.25) |
| +3 (display) | ~24px | `text-2xl` | Display headings, onboarding hero | `leading-tight` (1.25) |

**Intentional exception:** nav rail labels at `text-[9px]` — below the
scale floor. Justified by the 60px rail width constraint; the label is
always paired with an 18px icon and never standalone.

### Line-height by role

- **Headings** (display, title, chapter): `leading-tight` (1.25). Tight
  because headings are short, semibold, and don't need inter-line breathing.
- **Mono labels** (micro, nano): `leading-tight` to `leading-none`. Single
  lines only; never wrap.
- **Body prose** (base): `leading-relaxed` (1.6). Multi-line descriptions
  and hints need generous inter-line spacing for sustained reading.
- **Compact data** (base in tables/lists): `leading-snug` (1.375). Readable
  without the vertical cost of `leading-relaxed`.

### Font stacks

| Role | Stack | Rationale |
|---|---|---|
| **Mono** (primary) | JetBrains Mono → IBM Plex Mono → system mono | Ligature-free coding font with clear `0O`, `1l` disambiguation; IBM Plex as fallback for similar x-height |
| **Sans** (secondary) | Geist → system-ui → sans-serif | Geometric sans matching the app's modern density aesthetic; system-ui for zero-FOUT when Geist isn't loaded |

### Token → scale mapping (ui-tokens.ts)

| Token | Scale step | Tailwind class | Face |
|---|---|---|---|
| `headingDisplay` | +3 (display) | `text-2xl` | sans |
| `headingChapter` | +1 (chapter) | `text-base` | sans |
| `bodyMuted` | 0 (base) | `text-sm` | sans |
| `marginalia` | −1 (micro) | `text-xs` | sans |
| `labelSmallCap` | −1 (micro) | `text-xs` | mono |
| `labelMicro` | −2 (nano) | `text-mono-mini` | mono |
| `chapterIndex` | −1 (micro) | `text-xs` | mono |
| `chip` / `chipActive` | −2 (nano) | `text-mono-mini` | mono |

## Motion system

### Decision tree

Before adding any animation, answer one of these questions. If none
applies, **do not animate**.

| Question | If yes → use | Example |
|---|---|---|
| Where did this come from / go to? | `spring` | Nav indicator sliding, shared-layout panel |
| What content just changed? | `flow` (fade + slide) | Surface transition, section reveal, page swap |
| Was my action received? | `swift` | Button press, toggle flip, hover feedback |
| Is a value still resolving? | `springSoft` | Number ticker counting up, progress bar easing |

### Preset derivation

| Preset | Duration / params | UX role | Derivation |
|---|---|---|---|
| `swift` | 140ms, ease `[0.4, 0, 0.2, 1]` | Acknowledgement | Apple HIG recommends 200-300ms for macOS; we use 140ms (faster) because ministr is keyboard-first and feedback must feel instant. The easing is Material "standard" (decelerate-dominant). |
| `flow` | 240ms, ease `[0.22, 1, 0.36, 1]` | Content transition | Long enough to orient ("what changed?") but short enough to never feel sluggish. The easing is a strong deceleration curve — fast entry, gentle settle. |
| `spring` | stiffness: 420, damping: 36, mass: 0.9 | Spatial movement | Critically-damped spring (ζ ≈ 0.88). Reaches target in ~180ms with no visible overshoot — physicality without bounce. Used for shared-layout animations where an element moves between positions. |
| `springSoft` | stiffness: 210, damping: 30 | Value change | Slightly underdamped (ζ ≈ 0.65). Slower arrival (~300ms) with a barely-perceptible ease-past communicates "this value is still resolving." Used for number tickers and progress indicators. |

### Spring physics note

The damping ratio ζ = damping / (2 × √(stiffness × mass)):
- `spring`: 36 / (2 × √(420 × 0.9)) = 36 / (2 × 19.44) = 36/38.88 ≈ **0.93** (overdamped — no bounce)
- `springSoft`: 30 / (2 × √(210 × 1.0)) = 30 / (2 × 14.49) = 30/28.98 ≈ **1.03** (also critically damped at mass=1)

Both springs are at or above critical damping — no visible oscillation.
This is intentional: bouncy springs signal playfulness (wrong for a
professional tool). The difference is **arrival speed**: spring arrives
fast (snappy panel transitions), springSoft arrives slowly (gentle
value changes).

### Variant catalog (motion.ts exports)

| Variant | Built from | Use for |
|---|---|---|
| `popIn` | scale 0.95→1 + opacity, `flow` timing | Modal/dialog appearance |
| `scrim` | opacity 0→1, `flow` timing | Backdrop overlay behind modals |
| `fadeRise` | y: 6→0 + opacity, `flow` timing | Page-level content entry |
| `slideOver` | x: 12→0, `flow` timing | Side panel slide-in |
| `listContainer` | staggerChildren: 0.04s | Parent of staggered list |
| `listItem` | y: 8→0 + opacity, `flow` timing | Individual list item entry |

### Reduced motion

`prefersReducedMotion()` (in `motion.ts`) queries
`prefers-reduced-motion: reduce`. When active:

- **All Framer Motion animations** should use `duration: 0` (instant).
  The utility exists; enforcement is per-component via conditional
  `transition` props.
- **CSS transitions** (`transitionInteractive`) remain — 150ms color
  transitions are acceptable under reduced motion (WCAG: "motion that
  creates the illusion of movement" is what must be suppressed, not
  instantaneous state changes).
- **No animation should convey information that's lost without motion.**
  If a spring animation shows "this element moved from A to B," the
  element must also be identifiable at B without having seen the motion.

---

## Color system

### Perceptual depth model (dark mode)

Dark surfaces use **luminance stepping** to communicate depth. Lower
luminance = further back; higher = closer to the user. Four tiers:

| Tier | Token | Hex (dark) | Role |
|---|---|---|---|
| Deepest | `bg-background` | `#16130E` | App chrome, behind all content |
| Recessed | `bg-surface-sunken` | `#100E0A` | Inset trays, code blocks, grouped content |
| Default | `bg-surface` | `#1E1B15` | Standard content background (panels, cards) |
| Elevated | `bg-surface-overlay` | `#28241C` | Hover states, dropdowns, active nav items |

Luminance delta between adjacent tiers: **~0.7%** — subtle but perceivable
in a dark environment. The warm undertone (amber-shifted neutrals) prevents
the "cold terminal" feel while maintaining professional restraint.

### Contrast targets (WCAG 2.1)

| Text tier | Target | Measured (on `bg-surface`) | Status |
|---|---|---|---|
| `text-text` (primary) | **≥7:1** (AAA) | **14.3:1** | Pass |
| `text-text-muted` (secondary) | **≥4.5:1** (AA) | **9.1:1** | Pass (exceeds AAA) |
| `text-text-dim` (tertiary) | **≥3:1** (non-text/large) | **3.8:1** | Acceptable* |
| `accent` (interactive) | **≥3:1** (non-text) | **8.0:1** | Pass (exceeds AAA) |

*\*`text-dim` is intentionally below AA (4.5:1) for normal text. Justification:
it is NEVER used for critical information, always accompanies a more prominent
element (a label next to a value, a timestamp next to a heading), and
communicates "this is auxiliary — ignore it until you specifically need it."
If text-dim content were important enough to require AA contrast, it should
use `text-text-muted` instead.*

### Accent + semantic tones

| Color | Token | Hex (dark) | Contrast on surface | Use |
|---|---|---|---|---|
| Amber | `accent` | `#F59E0B` | 8.0:1 | Primary interactive: active nav, links, focus rings |
| Red | `danger` | `#ef5d68` | 5.3:1 | Errors, destructive actions (always paired with icon) |
| Green | `success` | `#3fcf8e` | 8.6:1 | Connected, healthy, complete |
| Yellow | `warning` | `#f2c14e` | 10.2:1 | Stale, degraded, attention-needed |

All tones exceed 3:1 for non-text UI indicators. `danger` at 5.3:1 passes
AA for normal text but not AAA — acceptable because danger text always
appears with AlertTriangle icon (shape backup for color-blind users).

### Color-blind safety rules

1. **Never color alone.** Every status indicator pairs color with a
   distinguishing shape: StatusDot + pulse pattern, Badge + text label,
   ErrorCallout + AlertTriangle icon, Progress + percentage text.
2. **Red/green never adjacent without shape.** Success and danger can appear
   in the same view but are always distinguishable by icon (CheckCircle vs
   AlertTriangle) or text label, not just hue.
3. **Accent is one hue.** The amber accent is a single hue used consistently
   — no blue links, no purple visited, no teal CTAs. One accent = one
   meaning = "this is interactive or active."

---

## The rule

Build only from:

1. **Primitives** in `src/components/ui/*` (see inventory below).
2. **Role tokens** in `src/lib/ui-tokens.ts` — headings, labels, surfaces,
   borders, dividers, chips, `transitionInteractive`, `focusRing`.
3. **Motion presets** in `src/lib/motion.ts` — `swift/flow/spring`,
   `popIn`, `scrim`, `fadeRise`, `listContainer/listItem`.
4. **Theme tokens** from `app.css` via Tailwind (`bg-surface`,
   `border-border`, `text-text-dim`, `rounded-md|lg`, `shadow-sm|md|lg`,
   the accent/tone colors).

Do **not** hand-roll a bordered box, chip, stat, or modal — use the
primitive. Same role = same component = same look everywhere.

## Navigation architecture

The app uses **two-level navigation**:

**Level 1 — Nav rail** (`Sidebar.tsx`, 60px): 6 icon+label destinations
with `layoutId` active indicator, `whileTap` scale, and accent glow.
Always visible. Keyboard chords: `g a` (Ask), `g p` (Projects),
`g s` (Sessions), `g c` (Cloud), `g e` (Explore), `g ,` (Settings).

**Level 2 — SurfaceSidebar** (`ui/surface-sidebar.tsx`): used by
multi-section surfaces. 200px sidebar at `@min-[900px]/surface:`,
horizontal tab bar below that. `AnimatePresence mode="wait"` on content.

| Surface | Level 2? | Why |
|---|---|---|
| Ask | no | Single search-first experience |
| Projects | no | Master-detail, single view |
| Sessions | no | Responsive card grid, single view |
| Cloud | **SurfaceSidebar** (6 sections) | Connection, Corpora, API Keys, Webhooks, Usage, Sessions |
| Explore | **SurfaceSidebar** (4 sections) | Server, Logs, Explorer, Playground |
| Settings | **SurfaceSidebar** (3 sections) | General, AI assistants, About |

**Rule:** use SurfaceSidebar when a surface has 3+ distinct sections.
Single-purpose surfaces (Ask, Projects, Sessions) don't need one.

## Banned (lint-enforced)

- Arbitrary `tracking-[...]` except `tracking-[0.08em]` — use
  `labelMicro` / `labelSmallCap`.
- Arbitrary `rounded-[...]` / `shadow-[...]` — use role radii / `shadow-*`
  (only `shadow-[var(--glow-soft)]` is allowed).
- Raw hex / `rgb(` / `#rrggbb` colors in className — use theme tokens.
- `transition-none` on an interactive element — use `transitionInteractive`.
- `italic` — the design system is sans, dim, never italic.
- `border-2` for containers — hairline `border` + `border-border`.
- `rounded-none` — use `rounded-sm` (micro), `rounded-md` (controls),
  or `rounded-lg` (cards/panels).
- `.ministr-flash` — use `.ministr-pulse` or a designed fresh state.

## Radius roles

| Role | Class |
|---|---|
| pill / chip / dot | `rounded-full` |
| micro-element (histogram bar, inline indicator) | `rounded-sm` |
| control (button, input, small box) | `rounded-md` |
| card / panel / modal / tray | `rounded-lg` (hero: `rounded-xl`) |

## Layout — adaptive surfaces

Every top-level surface wraps in `<AdaptiveSurface>` (or applies the
`surfaceContainer` token). This establishes a **named container query
context** (`@container/surface`) so children respond to their actual
allocated width, not the viewport.

| Breakpoint prefix | Triggers at | Use for |
|---|---|---|
| `@min-[600px]/surface:` | >=600px | Multi-column form rows |
| `@min-[900px]/surface:` | >=900px | Sidebar + content layouts |
| `@min-[1200px]/surface:` | >=1200px | 2-col card grids, side-by-side panels |

**Content width tokens** (in `ui-tokens.ts`):

| Token | Meaning |
|---|---|
| `contentNarrow` | `max-w-3xl mx-auto` — prose, forms, about panels |
| `contentWide` | No cap — grids, master-detail, dashboards |
| `contentAdaptive` | Narrow below 900px, wide above (within the container) |

## Visual containment

| Pattern | Class | Use for |
|---|---|---|
| **Recessed tray** | `bg-surface-sunken rounded-lg p-4` | Groups of PrefRows, MetaRows, action grids |
| **ContentTray** | `<ContentTray>` / `<ContentTray compact>` | Visual containment for sub-sections |
| **Card** | `<Card>` | Standalone elevated container |

No bordered Zone-style boxes. No bg-surface card-within-card nesting.

## Error handling

Use `<ErrorCallout message={...} />` for inline error display. Optional
`title` for a bold heading, optional `action` for a retry button.
Visual: `rounded-lg border-danger/40 bg-danger/5` + AlertTriangle icon.

Do **not** hand-roll error `<div>`s with `border-danger` and
`AlertTriangle` — the pattern is encoded in the primitive.

## Accessibility

- Every clickable non-`<button>` element needs `role="button"`,
  `tabIndex={0}`, and `onKeyDown` (Enter/Space).
- Every bare `<button>` needs `focus-visible:outline-2
  focus-visible:outline-accent` (or the `focusRing` token).
- Every dialog/overlay uses `useDialog` for Escape-to-close, focus trap,
  and focus restore.

## Component inventory

31 primitives in `src/components/ui/`:

| Component | File | Purpose |
|---|---|---|
| ActivityFeed | `activity-feed.tsx` | Scrollable event timeline |
| AdaptiveSurface | `adaptive-surface.tsx` | Container query wrapper (`@container/surface`) |
| Badge | `badge.tsx` | Inline label pill (variant: default/muted/accent/success/warning/danger) |
| BrutalIcons | `brutal-icons.tsx` | Hand-drawn icon set (pin, branch, etc.) |
| BudgetBar | `budget-bar.tsx` | Horizontal budget utilization bar |
| BudgetRing | `budget-ring.tsx` | Circular budget gauge (SVG) |
| Button | `button.tsx` | Primary interactive control (variant: default/outline/ghost/danger) |
| Card | `card.tsx` | Elevated container with optional hover lift |
| ChipGroup / Chip | `chip-group.tsx` | Toggleable pill group for filters |
| CoherenceFeed | `coherence-feed.tsx` | Coherence event stream |
| ConfirmDialog | `confirm-dialog.tsx` | Modal with optional type-to-confirm token |
| ContentTray | `content-tray.tsx` | Recessed visual containment (`bg-surface-sunken rounded-lg`) |
| CorpusChip | `corpus-chip.tsx` | Corpus selector pill |
| CorpusSelect | `corpus-select.tsx` | Corpus dropdown selector |
| Disclosure | `disclosure.tsx` | Collapsible section with chevron |
| EmptyState | `empty-state.tsx` | Card-based empty placeholder (icon + title + hint + action) |
| ErrorCallout | `error-callout.tsx` | Inline error display (AlertTriangle + message + optional title/action) |
| FilterPill | `filter-pill.tsx` | Removable filter indicator |
| H1 | `heading.tsx` | Top-level surface heading |
| LabeledCard | `labeled-card.tsx` | Card with a bold label header |
| LabeledRow | `labeled-row.tsx` | Compact key-value row |
| MetricTile | `metric-tile.tsx` | Stat display (value + label + optional sparkline) |
| NumberTicker | `number-ticker.tsx` | Animated counting number |
| Progress | `progress.tsx` | Horizontal progress bar (toned, optional glow) |
| Sparkline | `sparkline.tsx` | Tiny inline SVG trend chart |
| StatusDot | `status-dot.tsx` | Colored dot indicator (pulse: off/live/once) |
| SurfaceSidebar | `surface-sidebar.tsx` | Level-2 sidebar nav + tab bar + animated content |
| Toggle / ToggleRow | `toggle.tsx` | On/off switch with label + description |
| TokenEconomicsBar | `token-economics-bar.tsx` | Stacked horizontal bar for token budget breakdown |
| TurnBlock | `turn-block.tsx` | Conversation turn container (Ask surface) |
| VitalCard | `vital-card.tsx` | Key metric card with tone coloring |

## Surface inventory

6 nav-rail destinations, each owning its own layout:

| Surface | File(s) | Nav Level 2 | Layout pattern |
|---|---|---|---|
| **Ask** | `ask/AskSurface.tsx` | none | Container queries (`@container/page`), sidebar at 1180px+ |
| **Projects** | `ProjectsSurface.tsx` | none | Master-detail flex, `AnimatePresence` staggered list |
| **Sessions** | `SessionsSurface.tsx` | none | Responsive grid `grid-cols-1 md:2 xl:3`, animated cards |
| **Cloud** | `CloudPanel.tsx` | SurfaceSidebar (6) | Per-section renders: Connection, Corpora, API Keys, Webhooks, Usage, Sessions |
| **Explore** | `ExploreSurface.tsx` | SurfaceSidebar (4) | Each tool (Server, Logs, Explorer, Playground) renders full-width |
| **Settings** | `SettingsSurface.tsx` | SurfaceSidebar (3) | General (PrefRows in trays), AI assistants (client list), About (action grid) |

**Sub-surfaces** (rendered inside a parent surface, not nav-rail destinations):

| Component | Parent | Purpose |
|---|---|---|
| GeneralSettings | Settings | Preference rows in recessed trays |
| AiAssistantsPanel | Settings | MCP client connect/test rows |
| AboutPanel | Settings | Version, maintenance actions, app info |
| ServerSettings | Explore | Daemon status MetaRows + diagnostics |
| LinkedProjectsPanel | Projects | Cross-project linking manager |
| ProjectSessions | Projects | Session cards for the selected project |

## Adding a new pattern

If no primitive/token fits, **add one** to `ui/*` or `ui-tokens.ts` and use
it — never inline a one-off. That keeps the contract closed.
