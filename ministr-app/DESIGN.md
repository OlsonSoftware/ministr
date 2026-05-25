# ministr-app — Design contract (v2)

This is the **single source of truth** for UI consistency. Every component
must obey it; `pnpm design:lint` enforces the hard rules in CI.

## The rule

Build only from:

1. **Primitives** in `src/components/ui/*` — `Card`, `Badge`, `Button`,
   `StatusDot`, `Progress`, `MetricTile`, `EmptyState`, `NumberTicker`,
   `Sparkline`, `BudgetRing`, `ContentTray`, `Toggle`, `ConfirmDialog`, …
2. **Role tokens** in `src/lib/ui-tokens.ts` — headings, labels, surfaces,
   borders, dividers, chips, `transitionInteractive`, `focusRing`.
3. **Motion presets** in `src/lib/motion.ts` — `swift/flow/spring`,
   `popIn`, `scrim`, `fadeRise`, `listContainer/Item`.
4. **Theme tokens** from `App.css` via Tailwind (`bg-surface`,
   `border-border`, `text-text-dim`, `rounded-md|lg`, `shadow-sm|md|lg`,
   the accent/tone colors).

Do **not** hand-roll a bordered box, chip, stat, or modal — use the
primitive. Same role → same component → same look everywhere.

## Banned (lint-enforced)

- Arbitrary `tracking-[…]` → use `labelMicro` / `labelSmallCap` (0.08em).
- Arbitrary `rounded-[…]` / `shadow-[…]` → use role radii / `shadow-*`
  (the only allowed arbitrary is `shadow-[var(--glow-soft)]`).
- Raw hex / `rgb(`/`#rrggbb` colors in className → use theme tokens.
- `transition-none` on an interactive element → use
  `transitionInteractive`.
- `font-serif` + `italic` "marginalia" voice → use `marginalia` /
  `bodyMuted` (Cockpit is sans, dim, not italic serif).
- `border-2` for containers → hairline `border` + `border-border`.
- `.ministr-flash` → `.ministr-pulse` or a designed fresh state.

## Radius roles

| Role | Class |
|---|---|
| pill / chip / dot | `rounded-full` |
| control (button, input, small box) | `rounded-md` |
| card / panel / modal | `rounded-lg` (hero `rounded-xl`) |

## Layout — adaptive surfaces

Every top-level surface wraps in `<AdaptiveSurface>` (or applies the
`surfaceContainer` token). This establishes a **named container query
context** (`@container/surface`) so children respond to their actual
allocated width, not the viewport.

| Breakpoint prefix | Triggers at | Use for |
|---|---|---|
| `@min-[600px]/surface:` | ≥600px | Multi-column form rows |
| `@min-[900px]/surface:` | ≥900px | Sidebar + content layouts |
| `@min-[1200px]/surface:` | ≥1200px | 2-col card grids, side-by-side panels |

**Content width tokens** (in `ui-tokens.ts`):

| Token | Meaning |
|---|---|
| `contentNarrow` | `max-w-3xl mx-auto` — prose, forms, about panels |
| `contentWide` | No cap — grids, master-detail, dashboards |
| `contentAdaptive` | Narrow below 900px, wide above (within the container) |

**When to use which:**
- **Already full-width** (ProjectsSurface, SessionsSurface, AskSurface):
  wrap in `AdaptiveSurface` for consistency but don't change inner layout.
- **Currently narrow** (Settings, Cloud, Server): wrap in
  `AdaptiveSurface` and switch from hardcoded `max-w-*` to
  `contentAdaptive` or responsive container-query grids.
- **Prose content** (About panel, descriptions): keep `contentNarrow`
  even inside a wider layout — reading width matters.

## Adding a new pattern

If no primitive/token fits, **add one** to `ui/*` or `ui-tokens.ts` and use
it — never inline a one-off. That keeps the contract closed.
