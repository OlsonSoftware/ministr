import { Card } from "./card";
import { cn } from "../../lib/utils";
import { labelSmallCap } from "../../lib/ui-tokens";

interface VitalCardProps {
  /** Small-caps title shown in the header. */
  title: string;
  /** Optional one-line subtitle below the title. */
  subtitle?: string;
  /** Optional content rendered on the right side of the header (badge, chip). */
  right?: React.ReactNode;
  /** When true, replaces `children` with a centered `emptyLabel` placeholder. */
  empty?: boolean;
  /** Text shown in the empty placeholder. */
  emptyLabel?: string;
  /** How the body content is laid out:
   *  - `"default"` — children flow naturally (Overview-style).
   *  - `"center"`  — children are centered horizontally (SessionDashboard-style). */
  layout?: "default" | "center";
  children: React.ReactNode;
}

/**
 * Standard "vital" card used on the Overview and SessionDashboard pages
 * to show a single headline metric with optional subtitle and right-slot.
 *
 * Replaces the duplicated `VitalCard` definitions that previously lived
 * in both pages with ~70% overlapping JSX.
 */
export function VitalCard({
  title,
  subtitle,
  right,
  empty = false,
  emptyLabel,
  layout = "default",
  children,
}: VitalCardProps) {
  return (
    <Card hover="lift" className="p-4">
      <div className="flex items-start justify-between gap-2 mb-3">
        <div>
          <h3 className={labelSmallCap}>{title}</h3>
          {subtitle && (
            <p className="text-[11px] text-text-dim mt-0.5">{subtitle}</p>
          )}
        </div>
        {right}
      </div>
      {empty ? (
        <div className="flex h-[118px] items-center justify-center">
          <span className="text-xs text-text-dim">{emptyLabel}</span>
        </div>
      ) : (
        <div className={cn(layout === "center" && "flex items-center justify-center")}>
          {children}
        </div>
      )}
    </Card>
  );
}
