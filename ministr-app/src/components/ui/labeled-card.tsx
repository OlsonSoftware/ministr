import { Card } from "./card";
import { cn } from "../../lib/utils";
import { labelSmallCap } from "../../lib/ui-tokens";

interface LabeledCardProps {
  /** Small-caps title shown in the header. */
  title: string;
  /** Optional inline icon before the title. */
  icon?: React.ComponentType<{ className?: string }>;
  /** Optional content on the right side of the header (badge, live dot, etc.). */
  right?: React.ReactNode;
  /** Override the body padding (use `mono` style for compact code/ID blocks). */
  mono?: boolean;
  children: React.ReactNode;
}

/**
 * Compact data panel with a small-caps header. Used by the Overview side
 * panels and the ProjectDetail page where each section is a labeled
 * group of small rows rather than a prominent feature.
 *
 * Replaces the local `Section` and `SidePanel` helpers that duplicated
 * this pattern across ProjectDetail.tsx and Overview.tsx.
 */
export function LabeledCard({
  title,
  icon: Icon,
  right,
  mono = false,
  children,
}: LabeledCardProps) {
  return (
    <Card hover="lift" className={cn(mono && "p-3")}>
      <div className="flex items-center gap-1.5 mb-2.5">
        {Icon && <Icon className="h-3.5 w-3.5 text-text-dim" />}
        <h3 className={cn(labelSmallCap, "flex-1")}>{title}</h3>
        {right}
      </div>
      <div className="space-y-1.5">{children}</div>
    </Card>
  );
}
