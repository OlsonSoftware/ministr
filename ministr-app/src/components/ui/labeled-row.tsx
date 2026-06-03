import { cn } from "../../lib/utils";

interface LabeledRowProps {
  label: string;
  value: React.ReactNode;
  /** Render the value in monospace with tabular numbers. */
  mono?: boolean;
  /** Add a horizontal divider below each row (Settings-style). */
  bordered?: boolean;
}

/**
 * Label/value row — mono tracked label + value. Hairline bottom divider
 * (§4) when `bordered` (Settings-style lists).
 */
export function LabeledRow({
  label,
  value,
  mono = false,
  bordered = false,
}: LabeledRowProps) {
  return (
    <div
      className={cn(
        "flex items-center justify-between text-xs",
        bordered && "py-1.5 border-b border-border-soft last:border-0",
      )}
    >
      <span className="font-mono tracking-[0.08em] text-xs text-text-dim">
        {label}
      </span>
      <span className={cn("text-text", mono && "font-mono tabular-nums")}>
        {value}
      </span>
    </div>
  );
}
