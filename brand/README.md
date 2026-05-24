# ministr brand assets

Single source of truth for the ministr brand identity. Consumed by
both `web/` (marketing site) and `ministr-app/` (Tauri desktop app).

## Files

| File | Purpose |
|------|---------|
| `logo.svg` | Amber-gradient square logo with inner cutout |
| `tokens.json` | Brand colors + wordmark config |

## Colors

| Token | Hex | Usage |
|-------|-----|-------|
| `brand.amber` | `#F59E0B` | Primary brand color — wordmark dot, logo tint |
| `brand.amber_hover` | `#FBB833` | Hover state for brand elements |
| `brand.gradient_start` | `#F8AC18` | Logo gradient start |
| `brand.gradient_end` | `#FF9900` | Logo gradient end |

## Rules

- The logo gradient colors and amber hex are the same everywhere.
- Each surface (web, desktop) keeps its own design system and accent
  color. Only the brand identity (logo + amber + wordmark) crosses.
- Do not change these values without updating both consumers.
