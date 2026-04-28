import { cn } from "../../lib/utils";

interface LabeledRowProps {
  label: string;
  value: React.ReactNode;
  /** Render the value in monospace with tabular numbers. */
  mono?: boolean;
  /** Add a horizontal divider below each row (Settings-style data list).
   *  Defaults to false (ProjectDetail-style packed rows). */
  bordered?: boolean;
}

/**
 * A label/value row used inside compact data panels (Settings sections,
 * ProjectDetail cards). Replaces the local `Row` helpers that used to
 * live in both files.
 *
 * Overview.tsx keeps its own dl/dt/dd Row variant — the semantic
 * description-list markup there is intentional and doesn't fit this
 * span-based shape.
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
        bordered && "py-1 border-b border-border/40 last:border-0",
      )}
    >
      <span className="text-text-muted">{label}</span>
      <span className={cn("text-text", mono && "font-mono tabular-nums")}>
        {value}
      </span>
    </div>
  );
}
