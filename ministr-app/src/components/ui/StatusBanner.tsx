import type { ReactNode } from "react";
import { TrustMark } from "./TrustMark";
import type { TrustState } from "./trust";

/**
 * StatusBanner — the plain-English trust headline (DESIGN.md §7).
 * The MARK carries the tone; the headline stays ink (§2.4). The action
 * slot is where a cost-stating ActionChip lands ("Catch up · ~40s").
 */
export function StatusBanner({
  state,
  headline,
  sub,
  action,
}: {
  state: TrustState;
  headline: string;
  sub?: string;
  action?: ReactNode;
}) {
  return (
    <section
      aria-label={headline}
      className="flex items-start justify-between gap-4 rounded-lg border border-line bg-surface p-4"
    >
      <div className="flex items-start gap-3">
        <TrustMark state={state} className="mt-1 text-base" />
        <div>
          <h2 className="text-xl font-semibold tracking-tight text-ink">
            {headline}
          </h2>
          {sub ? <p className="mt-1 text-sm text-dim">{sub}</p> : null}
        </div>
      </div>
      {action ? <div className="shrink-0">{action}</div> : null}
    </section>
  );
}
