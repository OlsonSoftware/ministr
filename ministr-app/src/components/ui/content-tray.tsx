import { cn } from "../../lib/utils";

/**
 * Recessed tray — subtle inset background that groups content without
 * card-like borders or header strips. Used for preference rows, meta
 * rows, action grids, and other clustered content that needs visual
 * containment.
 */
export function ContentTray({
  children,
  className,
  compact,
}: {
  children: React.ReactNode;
  className?: string;
  compact?: boolean;
}) {
  return (
    <div
      className={cn(
        "bg-surface-sunken rounded-lg",
        compact ? "p-3" : "p-4",
        className,
      )}
    >
      {children}
    </div>
  );
}
