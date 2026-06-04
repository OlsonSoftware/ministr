/**
 * lens-frame — the shared lens-chrome grammar for the Explore facet.
 *
 * Every Explore lens (Bridges · Unused · Quality · Diagnostics · Changes) is
 * bespoke in its BODY but should read as ONE system in its CHROME. Before this,
 * each lens hand-rolled the same header frame, the same centered blinking
 * loading line, and the same centered empty-state wrapper — so they drifted.
 * This module is the single source of truth for that chrome:
 *
 *   • {@link LensHeader} — the glance header (toned icon + title, an inline
 *     glance stat line, lens-specific filter chips as children, a "what this
 *     answers" hint). Standardises the frame + title typography + hint; each
 *     lens still passes its own rich, colour-coded glance + filters.
 *   • {@link LensLoading} — the centered, blinking "<verb>_" loading line.
 *   • {@link LensEmpty} — the centered wrapper around the shared {@link EmptyState}
 *     atom (the empty / error / not-applicable state).
 *
 * This is the 2026 "UI stack" norm (Carbon, Uber Base, HPE all codify one
 * loading/empty/error/content template) applied to the lens vocabulary.
 */
import type { ComponentProps, ComponentType, ReactNode } from "react";
import { RefreshCw } from "lucide-react";
import { cn } from "../../lib/utils";
import { EmptyState } from "./empty-state";

/** A lens icon — the lucide-react icon component shape (className + strokeWidth). */
type LensIcon = ComponentType<{ className?: string; strokeWidth?: number }>;

/** The accent the lens header's icon + title take — usually the lens's identity
 *  colour, or a severity-driven tone (Diagnostics goes danger/warning/success). */
export type LensTone = "neutral" | "accent" | "warning" | "danger" | "success";

const TONE_CLASS: Record<LensTone, string> = {
  neutral: "text-text",
  accent: "text-accent",
  warning: "text-warning",
  danger: "text-danger",
  success: "text-success",
};

export interface LensHeaderProps {
  /** The lens's identity icon. */
  icon: LensIcon;
  /** The lens's UPPERCASE title (e.g. "Cross-language bridges"). */
  title: string;
  /** Icon + title accent. Defaults to the accent identity colour. */
  tone?: LensTone;
  /** The inline glance stat line (e.g. "N seams · M mechanisms"). Rendered in
   *  the standard muted style; inner toned spans still override per element. */
  glance?: ReactNode;
  /** The "what this answers" microcopy, in the standard hint style. */
  hint?: ReactNode;
  /** Lens-specific filter chips / facet rows, placed between glance and hint. */
  children?: ReactNode;
  /** When set, a refresh button is shown that re-runs the lens's analysis —
   *  these views are snapshots, so freshness must be one click away (2026). */
  onRefresh?: () => void;
  /** True while a re-run is in flight — spins the refresh icon + disables it. */
  refreshing?: boolean;
}

/**
 * The standard lens header frame. Owns the frame (padding, border, surface),
 * the title typography + tone, and the hint typography — so every lens header
 * lines up — while leaving the glance + filter content to each lens.
 */
export function LensHeader({
  icon: Icon,
  title,
  tone = "accent",
  glance,
  hint,
  children,
  onRefresh,
  refreshing = false,
}: LensHeaderProps) {
  return (
    <header className="shrink-0 border-b border-border-soft bg-surface px-4 py-3 space-y-2.5">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <div className={cn("flex items-center gap-2", TONE_CLASS[tone])}>
          <Icon className="h-4 w-4" strokeWidth={2} />
          <span className="font-mono text-xs font-bold uppercase tracking-[0.08em]">
            {title}
          </span>
        </div>
        {glance != null && (
          <span className="font-mono text-mono-mini text-text-dim">{glance}</span>
        )}
        {onRefresh && (
          <button
            type="button"
            onClick={onRefresh}
            disabled={refreshing}
            title="Re-run this analysis"
            aria-label="Re-run this analysis"
            className="ml-auto grid h-6 w-6 shrink-0 self-center place-items-center rounded-md border border-border-soft text-text-muted hover:border-border hover:text-text disabled:opacity-60 cursor-pointer transition-colors duration-150 ease-out"
          >
            <RefreshCw
              className={cn("h-3 w-3", refreshing && "animate-spin")}
              strokeWidth={2}
            />
          </button>
        )}
      </div>
      {children}
      {hint != null && (
        <p className="font-mono text-mono-micro text-text-dim">{hint}</p>
      )}
    </header>
  );
}

/** The centered, blinking loading line every lens shares while it fetches. */
export function LensLoading({ label }: { label: string }) {
  return (
    <div className="grid h-full place-items-center">
      <span className="font-mono text-sm text-text-dim">
        {label}
        <span className="ministr-blink">_</span>
      </span>
    </div>
  );
}

/** The centered empty / error state — the shared {@link EmptyState} atom in the
 *  lens's full-height frame. Same props as EmptyState. */
export function LensEmpty(props: ComponentProps<typeof EmptyState>) {
  return (
    <div className="grid h-full place-items-center p-6">
      <EmptyState {...props} />
    </div>
  );
}

/** A labelled re-run button — the CTA form of the refresh affordance, for an
 *  empty state (2026: an empty state earns a clear next action). */
export function LensRerunButton({
  onRefresh,
  refreshing = false,
  label = "Re-run",
}: {
  onRefresh: () => void;
  refreshing?: boolean;
  label?: string;
}) {
  return (
    <button
      type="button"
      onClick={onRefresh}
      disabled={refreshing}
      className="inline-flex items-center gap-1.5 rounded-md border border-border-soft bg-surface px-2.5 py-1 font-mono text-mono-mini font-semibold uppercase tracking-[0.06em] text-text-muted hover:border-border hover:text-text disabled:opacity-60 cursor-pointer transition-colors duration-150 ease-out"
    >
      <RefreshCw
        className={cn("h-3 w-3", refreshing && "animate-spin")}
        strokeWidth={2}
      />
      {refreshing ? "Re-running" : label}
    </button>
  );
}
