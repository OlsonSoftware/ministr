# ministr-app — Design contract (v2)

This is the **single source of truth** for UI consistency. Every component
must obey it; `pnpm design:lint` enforces the hard rules in CI.

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
