import { cn } from "../../lib/utils";

interface ToggleProps {
  enabled: boolean | null;
  onToggle: () => void;
  /** Accessible label. Required when there's no visible text label. */
  ariaLabel?: string;
}

/**
 * Bare on/off switch. Pass `enabled = null` while async state is pending
 * (renders disabled with a wait cursor).
 */
export function Toggle({ enabled, onToggle, ariaLabel }: ToggleProps) {
  return (
    <button
      onClick={onToggle}
      disabled={enabled === null}
      role="switch"
      aria-checked={!!enabled}
      aria-label={ariaLabel}
      className={cn(
        "relative h-6 w-10 shrink-0 rounded-full transition-colors duration-150 cursor-pointer",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-accent-ring)]",
        enabled ? "bg-accent" : "bg-surface-overlay",
        enabled === null && "opacity-50 cursor-wait",
      )}
    >
      <span
        className={cn(
          "absolute top-0.5 left-0.5 h-5 w-5 rounded-full bg-white shadow-sm transition-transform duration-150",
          enabled && "translate-x-4",
        )}
      />
    </button>
  );
}

interface ToggleRowProps {
  label: string;
  description?: string;
  enabled: boolean | null;
  onToggle: () => void;
}

/**
 * A label + optional description with a Toggle on the right. Used in
 * Settings sections.
 */
export function ToggleRow({
  label,
  description,
  enabled,
  onToggle,
}: ToggleRowProps) {
  return (
    <div className="flex items-start justify-between gap-4">
      <div className="flex-1">
        <p className="text-sm font-medium text-text">{label}</p>
        {description && (
          <p className="text-xs text-text-dim mt-0.5">{description}</p>
        )}
      </div>
      <Toggle enabled={enabled} onToggle={onToggle} ariaLabel={label} />
    </div>
  );
}
