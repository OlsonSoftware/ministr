# ministr desktop — design language v5 ("Clear Glass")

**Status:** v5.0 · 2026-06-11 · governs the GUI rewrite (UX-BLUEPRINT v4).
The previous language (command-deck v4) is archived with the old app on
branch `archive/app-v1` and does not apply to any new code.

## §1 · Thesis

The app is a **trust instrument**: it exists to answer "is my AI seeing my
code properly?" for someone who may never have heard the word *embedding*.
The design language therefore has one governing law:

> **Color is meaning, and trust is the only meaning color carries.**

Neutrals do all the furniture. The brand amber marks identity and liveness.
The trust tones (ok / stale / hidden / updating) are the ONLY other color
on screen — so when something needs you, it is the only colored thing in
view. Quiet until it isn't: mission control, not a dashboard.

## §2 · Principles

1. **Never color alone.** Every state = glyph (distinct shape) + word +
   tone. ✓ ⚠ ✗ ⟳ are shapes first, colors second (WCAG 1.4.1; validated
   against 2026 practice — see roadmap think:57).
2. **Plain words above the fold.** "Your AI sees your code — up to date."
   Internals vocabulary (vectors, tokens, embeddings) is BANNED outside
   `advanced`/`expert` disclosures. The lint can't catch prose; reviews must.
3. **Equal-weight honesty.** Good news and bad news share typography and
   size. Credibility is the product; curation reads as lying.
4. **The headline is ink.** Tone lives on the mark, never on sentence text —
   state-colored body text is how AA failures happen (recurring lesson from
   v4: text-danger-on-wash failed 3×).
5. **Displayed values never lie.** Animations may ease; numbers don't.
   Receipts restate recorded events 1:1.

## §3 · Color tokens (the complete palette)

Defined once in `src/app.css`; consumed ONLY via utility names. Raw hex in
a className is a lint failure.

| Token | Role | Light | Dark |
|---|---|---|---|
| `bg` | app background (paper / console) | `#F7F6F2` | `#0E0D0B` |
| `surface` | cards, rails | `#FFFFFF` | `#171511` |
| `sunken` | wells, code panes | `#EFEDE7` | `#0A0908` |
| `ink` | all primary text | `#1C1A16` | `#ECE8E0` |
| `dim` | secondary text (AA on bg+surface) | `#5C574E` | `#A8A193` |
| `line` | hairline borders | `#DDD9CF` | `#2A2721` |
| `brand` | identity + liveness (dot, pulse) | `#B45309` | `#F5A623` |
| `ok` | trust: current / win | `#1A7F37` | `#3FB950` |
| `stale` | trust: behind / heads-up | `#9A6700` | `#D29922` |
| `hidden` | trust: excluded (deliberately neutral) | = `dim` | = `dim` |
| `ok-wash` / `stale-wash` | non-text tint behind a stated state | `#E9F3EB` / `#F8F0DE` | `#13211A` / `#262008` |

Rules: washes are never the only signal and never carry text other than
`ink`/`dim`. `hidden` being neutral is deliberate — exclusion is a choice,
not a problem.

## §4 · Surfaces & depth

Flat-first: `bg` → `surface` (1px `line` border, `rounded-lg`) → `sunken`
wells. One shadow tier (`shadow-sm`) for raised moments; arbitrary shadows
are a lint failure. No glass/blur in v5.0 (revisit only with a token).

## §5 · Type

System stack (`font-sans` default; `font-mono` for file paths, hashes and
times ONLY). Scale: `text-xs` metadata · `text-sm` body (default) ·
`text-xl font-semibold tracking-tight` for the StatusBanner headline ·
`text-2xl` reserved for the connect-flow hero. Uppercase only in micro
rail labels (`text-xs` + `tracking-[0.08em]`), never prose. No italics.

## §6 · Motion

Two sanctioned animations, both opacity/transform-based and guarded by
`prefers-reduced-motion`:
- `pulse-live` — the LiveDot / updating-⟳ breath (2s ease, 1 → .45).
- `beat-sweep` — Beat's indeterminate progress sweep.
Anything else needs a new token here first. Reduced motion ⇒ settled UI;
values still update.

## §7 · The atoms (`src/components/ui/`)

| Atom | Speaks | Notes |
|---|---|---|
| `TrustMark` | one trust state | glyph+tone+aria; the vocabulary's letterform |
| `StatusBanner` | the plain-English headline | mark + ink headline + dim sub + action slot |
| `TreeRow` | one file's truth | mono name, mark, note, optional action; indent by level |
| `Receipt` | one restated event | time + sentence; win/heads-up at EQUAL weight |
| `LiveDot` | presence | brand pulse, motion-safe, labeled |
| `Beat` | indexing progress | plain-words sentence + sweep |
| `RailSection` / `RailRow` | config-where-you-look | label + rows; the only uppercase |
| `ActionChip` | the one button | `primary` (brand) / `quiet` variants |
| `Brand` | identity | logo + wordmark + amber dot |

Every atom has a story; every story passes axe in light AND dark (the
Vitest browser projects). A new atom without a story does not merge.

## §8 · Language register

Address the user as "you", their agent as "your AI". Name files, not
concepts ("login.tsx", not "3 documents"). Actions state their cost
("Catch up · ~40s"). Never blame ("you forgot to…"); state and offer.

## §9 · Definition of Done (mechanical)

`pnpm test` (axe both themes) + `pnpm design:lint` + `npx tsc --noEmit` +
`pnpm build`. The lint bans: raw hex/rgb in classNames, arbitrary shadows/
blurs/durations, `font-serif`, italics, `border-2`, `rounded-none`,
`transition-none`. Allowed definition sites: `src/app.css`,
`src/components/ui/trust.ts`.
