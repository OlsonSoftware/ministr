/**
 * VizFrame — the shared command-deck panel the data-viz suite renders in.
 *
 * Every bespoke visualization (ActivityPulse, DiffRipple, CodebaseConstellation,
 * …) sits in the SAME premium frame so the suite reads as one designed
 * instrument family, not a set of one-offs: the raised tier + an accent
 * lit-edge (a top accent border — the token-safe lit edge, never an arbitrary
 * shadow), an optional eyebrow (icon + mono uppercase accent label), an optional
 * right-aligned readout slot, and the chart as children. The chart internals are
 * each viz's own business; this only owns the frame.
 */
import { type ComponentType, type ReactNode } from "react";
import { cn } from "../../lib/utils";

export interface VizFrameProps {
  /** Eyebrow glyph — paired with `label`. Omit when a host section already
   *  labels the panel (e.g. the Observatory's "Structure" SectionLabel). */
  icon?: ComponentType<{ className?: string; strokeWidth?: number }>;
  /** Eyebrow text — mono, uppercase, accent-toned. */
  label?: string;
  /** Right-aligned readout (counts / rates). Renders full-width when there's no
   *  eyebrow, so a self-justified readout keeps its own layout. */
  readout?: ReactNode;
  children: ReactNode;
  className?: string;
}

const READOUT_CLS =
  "flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-mono-micro text-text-dim";

export function VizFrame({ icon: Icon, label, readout, children, className }: VizFrameProps) {
  const hasEyebrow = Boolean(Icon && label);

  return (
    <div
      className={cn(
        "flex flex-col gap-2 rounded-xl border border-border border-t-2 border-t-accent/50 bg-surface-raised px-4 py-3 shadow-sm",
        className,
      )}
    >
      {hasEyebrow ? (
        <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1.5">
          <div className="flex items-center gap-1.5 text-accent">
            {Icon && <Icon className="h-3.5 w-3.5" strokeWidth={2} />}
            <span className="font-mono text-mono-micro font-bold uppercase tracking-[0.12em]">
              {label}
            </span>
          </div>
          {readout && <div className={READOUT_CLS}>{readout}</div>}
        </div>
      ) : (
        readout && <div className={cn(READOUT_CLS, "justify-between")}>{readout}</div>
      )}
      {children}
    </div>
  );
}
