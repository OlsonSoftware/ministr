import { Card } from "./card";
import { cn } from "../../lib/utils";
import { labelSmallCap } from "../../lib/ui-tokens";

interface VitalCardProps {
  /** Mono uppercase title shown in the header. */
  title: string;
  /** Optional one-line subtitle below the title. */
  subtitle?: string;
  /** Optional content rendered on the right side of the header. */
  right?: React.ReactNode;
  /** When true, replaces `children` with a centered `emptyLabel` placeholder. */
  empty?: boolean;
  /** Text shown in the empty placeholder. */
  emptyLabel?: string;
  /** Body layout: `"default"` (natural flow) or `"center"` (centered). */
  layout?: "default" | "center";
  children: React.ReactNode;
}

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
            <p className="text-[0.6875rem] text-text-dim mt-0.5 font-mono">{subtitle}</p>
          )}
        </div>
        {right}
      </div>
      {empty ? (
        <div className="flex h-[118px] items-center justify-center">
          <span className="font-mono text-xs tracking-[0.05em] text-text-dim">
            {emptyLabel}
          </span>
        </div>
      ) : (
        <div className={cn(layout === "center" && "flex items-center justify-center")}>
          {children}
        </div>
      )}
    </Card>
  );
}
