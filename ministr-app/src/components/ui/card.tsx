import { forwardRef } from "react";
import { cn } from "../../lib/utils";

interface CardProps extends React.HTMLAttributes<HTMLDivElement> {
  /** Hover treatment: "none" (default), "lift" (faint surface shift on hover),
   *  or "accent" (lift with accent edge). The hard offset shadow is reserved
   *  for focused/active state — hover is just a contrast cue now. */
  hover?: "none" | "lift" | "accent";
  /** Use the sunken background (inset feel) instead of the raised surface. */
  sunken?: boolean;
}

/**
 * Field-manual card: hairline border, no shadow by default. Padding reduces
 * in compact density mode via the `[data-density="compact"]` selector.
 */
export const Card = forwardRef<HTMLDivElement, CardProps>(function Card(
  { className, hover = "none", sunken = false, ...props },
  ref,
) {
  return (
    <div
      ref={ref}
      className={cn(
        "border border-border-soft transition-none rounded-none",
        "p-4 [html[data-density=compact]_&]:p-2.5",
        sunken ? "bg-surface-sunken" : "bg-surface-raised",
        hover === "lift" && "hover:bg-surface-overlay hover:border-border",
        hover === "accent" && "hover:bg-surface-overlay hover:border-accent",
        className,
      )}
      {...props}
    />
  );
});
