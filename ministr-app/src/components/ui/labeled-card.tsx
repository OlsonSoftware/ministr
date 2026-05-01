import { Card } from "./card";
import { cn } from "../../lib/utils";
import { labelSmallCap } from "../../lib/ui-tokens";

interface LabeledCardProps {
  /** Mono uppercase title shown in the header. */
  title: string;
  /** Optional inline icon before the title. */
  icon?: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  /** Tint for the inline icon. `"dim"` (default) for side panels;
   *  `"accent"` for surfaces where the icon is the section's identity cue. */
  iconTone?: "accent" | "dim";
  /** Optional content on the right side of the header. */
  right?: React.ReactNode;
  /** Compact body padding. */
  mono?: boolean;
  children: React.ReactNode;
}

/**
 * Compact data panel with a mono uppercase header. Used by side panels
 * and the ProjectDetail page.
 */
export function LabeledCard({
  title,
  icon: Icon,
  iconTone = "dim",
  right,
  mono = false,
  children,
}: LabeledCardProps) {
  return (
    <Card hover="lift" className={cn(mono && "p-3")}>
      <div className="flex items-center gap-1.5 mb-2.5">
        {Icon && (
          <Icon
            className={cn(
              "h-3.5 w-3.5",
              iconTone === "accent" ? "text-accent" : "text-text-dim",
            )}
            strokeWidth={2.5}
          />
        )}
        <h3 className={cn(labelSmallCap, "flex-1")}>{title}</h3>
        {right}
      </div>
      <div className="space-y-1.5">{children}</div>
    </Card>
  );
}
