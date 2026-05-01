import type React from "react";
import { Card } from "./card";
import { cn } from "../../lib/utils";

interface EmptyStateProps {
  icon: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  title: string;
  hint?: React.ReactNode;
  /** Optional CTA rendered below the hint. Pass a Button or anchor. */
  action?: React.ReactNode;
  /** Use the accent-filled icon container (for places where the empty
   *  state itself is a primary call-to-action). Defaults to muted. */
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
          "grid place-items-center border border-border-soft mb-2",
          accent
            ? "h-14 w-14 bg-accent text-[var(--color-accent-fg-on)]"
            : "h-12 w-12 bg-surface-overlay text-text-muted",
        )}
      >
        <Icon className={accent ? "h-6 w-6" : "h-5 w-5"} strokeWidth={2} />
      </div>
      <p className="font-serif text-lg font-bold text-text leading-snug">
        {title}
      </p>
      {hint && (
        <p className="max-w-sm font-serif text-sm italic text-text-dim leading-snug">
          {hint}
        </p>
      )}
      {action && <div className="mt-3">{action}</div>}
    </Card>
  );
}
