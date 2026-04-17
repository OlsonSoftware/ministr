import { cn } from "../../lib/utils";

interface CardProps extends React.HTMLAttributes<HTMLDivElement> {
  /** Hover treatment: "none" (default), "lift" (shadow + border), or "accent" (iris ring). */
  hover?: "none" | "lift" | "accent";
  /** Use the sunken background (inset feel) instead of the raised surface. */
  sunken?: boolean;
}

export function Card({
  className,
  hover = "none",
  sunken = false,
  ...props
}: CardProps) {
  return (
    <div
      className={cn(
        "rounded-xl border border-border p-4 transition-all duration-150",
        sunken ? "bg-surface-sunken" : "bg-surface-raised",
        hover === "lift" &&
          "hover:border-border-hover hover:shadow-[0_4px_16px_rgb(0_0_0/0.08)] dark:hover:shadow-[0_4px_16px_rgb(0_0_0/0.4)]",
        hover === "accent" &&
          "hover:border-[var(--color-accent-ring)] hover:shadow-[0_0_0_3px_var(--color-accent-soft)]",
        className,
      )}
      {...props}
    />
  );
}
