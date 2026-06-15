import type { ReactNode } from "react";

/**
 * ShellHeader — the ONE top-chrome contract every screen fills
 * (gui-shell-consistent-chrome). Before this, the four roots hand-rolled
 * three different tops (Home: Brand + actions; Mirror/Feed: back + title;
 * Connect: none) so the app didn't read as one app. Now every screen slots
 * the same row: a `leading` identity-or-back affordance on the left, an
 * optional `title`/`subtitle`, and right-aligned `trailing` global actions.
 *
 * Calm furniture — neutrals only, no second hue, no added chrome weight. It
 * is a layout contract, not a new bar: the same flex row the screens already
 * used, named once so they stop drifting.
 */
export function ShellHeader({
  leading,
  title,
  subtitle,
  trailing,
}: {
  /** Left affordance: the Brand on roots, a BackButton on drill-ins. */
  leading?: ReactNode;
  /** Screen title (rendered as the page h1). Omit on identity-led roots
   *  where the Brand in `leading` already names the place. */
  title?: string;
  /** A quiet inline gloss on the title, e.g. "what your AI sees". */
  subtitle?: string;
  /** Right-aligned global actions (Settings, a per-screen action…). */
  trailing?: ReactNode;
}) {
  return (
    <div className="flex items-center gap-3">
      {leading}
      {title ? (
        <h1 className="text-xl font-semibold tracking-tight text-ink">
          {title}
          {subtitle ? (
            <span className="ml-2 text-sm font-normal text-dim">{subtitle}</span>
          ) : null}
        </h1>
      ) : null}
      {trailing ? (
        <div className="ml-auto flex items-center gap-4">{trailing}</div>
      ) : null}
    </div>
  );
}
