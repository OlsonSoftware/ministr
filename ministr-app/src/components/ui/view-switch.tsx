/**
 * ViewSwitch — the shared "two reads of the same surface" segmented control.
 *
 * One inset track holding N labelled tabs; the active tab lifts onto the
 * surface-overlay with the sanctioned --glow-soft lit edge. This is the
 * vocabulary the Explore lens toggle established and the data-viz surfaces
 * reuse to flip between a management list and a bespoke visualization
 * (Fleet: Grid|Map · Activity: Board|Tree). Keep the control itself identical
 * everywhere so the gesture reads the same across the app.
 */
import type { IconComponent } from "./icons";
import { cn } from "../../lib/utils";

export interface ViewOption<T extends string> {
  id: T;
  label: string;
  icon: IconComponent;
  /** Tooltip — "what this view answers". */
  hint?: string;
}

export interface ViewSwitchProps<T extends string> {
  value: T;
  onChange: (v: T) => void;
  options: ViewOption<T>[];
  /** Accessible name for the tablist (e.g. "Fleet view", "Activity view"). */
  ariaLabel: string;
}

export function ViewSwitch<T extends string>({
  value,
  onChange,
  options,
  ariaLabel,
}: ViewSwitchProps<T>) {
  return (
    <div
      role="tablist"
      aria-label={ariaLabel}
      className="inline-flex items-center gap-0.5 rounded-md border border-border-soft bg-surface-sunken p-0.5"
    >
      {options.map(({ id, label, icon: Icon, hint }) => {
        const active = value === id;
        return (
          <button
            key={id}
            type="button"
            role="tab"
            aria-selected={active}
            title={hint}
            onClick={() => onChange(id)}
            className={cn(
              "inline-flex items-center gap-1 rounded px-2 py-0.5 font-mono text-mono-mini font-semibold uppercase tracking-[0.06em] cursor-pointer transition-colors duration-150 ease-out",
              active
                ? "bg-surface-overlay text-text shadow-[var(--glow-soft)]"
                : "text-text-dim hover:text-text",
            )}
          >
            <Icon className="h-3 w-3" strokeWidth={2.25} />
            {label}
          </button>
        );
      })}
    </div>
  );
}
