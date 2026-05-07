import type React from "react";
import { cn } from "../../lib/utils";

/**
 * Single chip primitive for filter / view-mode / kind selectors.
 *
 * Replaces three earlier in-component reimplementations
 * (Bridge::FilterPill, SessionDashboard::FilterPill,
 * QueryPlayground::ViewToggle). Uses the role-mapping tokens from
 * `lib/ui-tokens.ts`: `containerDefault` / `containerActive` and
 * `surfacePanel` / `surfacePanelActive`. Always rounded-sm (control radius).
 *
 * Two surface tones:
 * - `mono` (default) — small mono uppercase tracked. Use when the label
 *   is a kind / language / count category.
 * - `sans` — sans medium-weight body. Use when the label is a control
 *   verb ("Live", "History", "Compact") or a phrase.
 *
 * Two sizes:
 * - `sm` (default) — h-auto with px-2 py-0.5; works inline in filter rows.
 * - `md` — h-9 px-2.5; pairs with form inputs of the same height.
 */
interface FilterPillProps {
  /** Mono-uppercase label. Either `label` or `children` must be set. */
  label?: string;
  /** Optional trailing count chip — only renders when > 0. */
  count?: number;
  /** Custom content (overrides `label`). Pick this for sans tone. */
  children?: React.ReactNode;
  active: boolean;
  disabled?: boolean;
  onClick: () => void;
  tone?: "mono" | "sans";
  size?: "sm" | "md";
  className?: string;
}

export function FilterPill({
  label,
  count,
  children,
  active,
  disabled = false,
  onClick,
  tone = "mono",
  size = "sm",
  className,
}: FilterPillProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "border cursor-pointer transition-none rounded-sm",
        size === "sm" ? "px-2 py-0.5" : "h-9 px-2.5",
        tone === "mono"
          ? "text-mono-mini font-mono font-semibold uppercase tracking-[0.05em]"
          : "font-sans text-sm font-medium",
        active
          ? "border-accent bg-surface-overlay text-text"
          : "border-border-soft bg-surface text-text-muted hover:border-border hover:text-text",
        disabled && "opacity-40 cursor-not-allowed",
        className,
      )}
    >
      {children ?? label}
      {typeof count === "number" && count > 0 && (
        <span className="ml-1 tabular-nums opacity-70">{count}</span>
      )}
    </button>
  );
}
