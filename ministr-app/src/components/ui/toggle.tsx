import { cn } from "../../lib/utils";

interface ToggleProps {
  enabled: boolean | null;
  onToggle: () => void;
  /** Accessible label. Required when there's no visible text label. */
  ariaLabel?: string;
}

/**
 * Brutalist on/off switch — labeled `[ON]`/`[OFF]` mono button.
 * Pass `enabled = null` while async state is pending (renders disabled).
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
        "inline-flex h-7 min-w-[60px] items-center justify-center border-2 border-border px-2 text-mono-mini font-mono font-semibold uppercase tracking-[0.05em] cursor-pointer transition-none",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
        enabled
          ? "bg-accent text-[var(--color-accent-fg-on)] shadow-sm"
          : "bg-surface text-text-muted",
        enabled === null && "opacity-50 cursor-wait",
      )}
    >
      {enabled === null ? "…" : enabled ? "ON" : "OFF"}
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
        <p className="text-sm font-semibold text-text">{label}</p>
        {description && (
          <p className="text-xs text-text-dim mt-0.5">{description}</p>
        )}
      </div>
      <Toggle enabled={enabled} onToggle={onToggle} ariaLabel={label} />
    </div>
  );
}
