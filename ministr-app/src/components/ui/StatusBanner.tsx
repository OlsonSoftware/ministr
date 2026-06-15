import type { ReactNode } from "react";
import { TrustMark } from "./TrustMark";
import type { TrustState } from "./trust";

/**
 * Card-level trust cue (Clear Glass v5 C5): the state is pre-attentive at
 * the whole-card scale, legible across a list WITHOUT reading the glyph.
 * A constant 2px left rail (transparent by default → no layout shift)
 * turns the stale tone and the card gets a faint stale-wash tint when a
 * project is BEHIND; a hidden project recedes onto bg-sunken; healthy and
 * updating stay quiet (DESIGN §7). Trust tones + neutrals only — never a
 * second hue, and the body text stays ink (§2.4).
 */
const CARD_CUE: Record<TrustState, string> = {
  ok: "bg-surface border-l-transparent",
  stale: "bg-stale-wash border-l-stale",
  updating: "bg-surface border-l-transparent",
  hidden: "bg-sunken border-l-transparent",
};

/**
 * StatusBanner — the plain-English trust headline (DESIGN.md §7).
 * The MARK carries the tone; the headline stays ink (§2.4). The action
 * slot is where a cost-stating ActionChip lands ("Catch up · ~40s"); the
 * footer slot is where a live instrument lands inside the same card
 * (e.g. the Indexing Instrument while a row is updating).
 */
export function StatusBanner({
  state,
  headline,
  sub,
  action,
  footer,
}: {
  state: TrustState;
  headline: string;
  sub?: string;
  action?: ReactNode;
  footer?: ReactNode;
}) {
  return (
    <section
      aria-label={headline}
      className={`rounded-lg border border-line border-l-2 p-4 ${CARD_CUE[state]}`}
    >
      <div className="flex items-start justify-between gap-4">
        <div className="flex items-start gap-3">
          <TrustMark state={state} className="mt-1 text-base" />
          <div>
            <h2 className="text-xl font-semibold tracking-tight text-ink">
              {headline}
            </h2>
            {sub ? <p className="mt-1 text-sm text-dim">{sub}</p> : null}
          </div>
        </div>
        {action ? <div className="relative z-10 shrink-0">{action}</div> : null}
      </div>
      {footer ? <div className="mt-3">{footer}</div> : null}
    </section>
  );
}
