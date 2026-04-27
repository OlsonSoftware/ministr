import type React from "react";
import { Card } from "./card";
import { cn } from "../../lib/utils";

interface EmptyStateProps {
  icon: React.ComponentType<{ className?: string }>;
  title: string;
  hint?: React.ReactNode;
  /** Optional CTA rendered below the hint. Pass a Button or anchor. */
  action?: React.ReactNode;
  /** Use the accent-tinted icon container (for places where the empty
   *  state itself is a primary call-to-action; e.g. "Add your first
   *  project"). Defaults to the muted treatment. */
  accent?: boolean;
  className?: string;
}

export function EmptyState({
  icon: Icon,
  title,
  hint,
  action,
  accent = false,
  className,
}: EmptyStateProps) {
  return (
    <Card
      className={cn(
        "flex flex-col items-center justify-center gap-2 py-10 px-6 text-center",
        className,
      )}
    >
      <div
        className={cn(
          "grid place-items-center rounded-xl mb-2",
          accent
            ? "h-14 w-14 bg-[var(--color-accent-soft)] text-accent"
            : "h-12 w-12 bg-surface-overlay text-text-dim",
        )}
      >
        <Icon className={accent ? "h-6 w-6" : "h-5 w-5"} />
      </div>
      <p className="text-sm font-medium text-text">{title}</p>
      {hint && <p className="max-w-sm text-xs text-text-dim">{hint}</p>}
      {action && <div className="mt-3">{action}</div>}
    </Card>
  );
}
