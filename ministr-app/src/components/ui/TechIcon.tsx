import type { CSSProperties } from "react";
import { techEntry } from "../../lib/techIcons";

/**
 * TechIcon — one detected technology (gui-card-tech-icons). Neutral ink at
 * rest (honours the Clear Glass "amber=identity-only" law as standing
 * chrome); animates to the tech's real brand colour on hover/focus — the
 * sanctioned interaction reward. The brand hex never appears as a literal
 * className (it rides a `--tech` CSS var), so the colour stays off the
 * standing surface and the identity gate stays green.
 *
 * The colour is driven by `group-hover` / `group-focus-within` on the
 * enclosing card (its overlay button covers the content, so a per-icon
 * `:hover` can't fire); `hover:` covers standalone use (e.g. Storybook).
 */
export function TechIcon({ slug }: { slug: string }) {
  const e = techEntry(slug);
  if (!e) return null;

  if (e.path && e.hex) {
    return (
      <span
        role="img"
        title={e.title}
        aria-label={e.title}
        style={{ "--tech": `#${e.hex}` } as CSSProperties}
        className="inline-flex shrink-0 text-dim transition-colors hover:text-[var(--tech)] group-hover:text-[var(--tech)] group-focus-within:text-[var(--tech)]"
      >
        <svg viewBox="0 0 24 24" className="size-4" fill="currentColor" aria-hidden>
          <path d={e.path} />
        </svg>
      </span>
    );
  }

  // No licensed mark (e.g. C#) — a calm neutral lettermark.
  return (
    <span
      role="img"
      title={e.title}
      aria-label={e.title}
      className="inline-flex h-4 min-w-4 shrink-0 items-center justify-center rounded-[3px] border border-line px-0.5 text-[9px] font-semibold text-dim"
    >
      {e.mark ?? e.title}
    </span>
  );
}
