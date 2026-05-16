import { forwardRef } from "react";
import { cn } from "../../lib/utils";

interface CardProps extends React.HTMLAttributes<HTMLDivElement> {
  /** Hover treatment: "none", "lift" (surface + shadow), or "accent"
   *  (lift with accent ring + glow). */
  hover?: "none" | "lift" | "accent";
  /** Use the sunken background (inset feel) instead of the raised surface. */
  sunken?: boolean;
}

/**
 * Cockpit card: tier-1 surface, hairline border, soft radius. Elevation
 * is communicated by shadow on hover/active (not a permanent hard
 * offset). Padding tightens in compact density mode.
 */
export const Card = forwardRef<HTMLDivElement, CardProps>(function Card(
  { className, hover = "none", sunken = false, ...props },
  ref,
) {
  return (
    <div
      ref={ref}
      className={cn(
        "border border-border rounded-lg",
        "transition-[background-color,box-shadow,border-color] duration-200 ease-out",
        "p-4 [html[data-density=compact]_&]:p-2.5",
        sunken ? "bg-surface-sunken" : "bg-surface",
        hover === "lift" &&
          "hover:bg-surface-overlay hover:border-border-hover hover:shadow-md",
        hover === "accent" &&
          "hover:border-accent hover:shadow-[var(--glow-soft)]",
        className,
      )}
      {...props}
    />
  );
});
