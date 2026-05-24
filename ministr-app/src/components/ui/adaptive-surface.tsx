import { cn } from "@/lib/utils";

/**
 * Container-query wrapper for top-level surfaces. Establishes a named
 * `@container/surface` so children can use Tailwind's container-query
 * prefixes to adapt to the actual allocated space (not the viewport).
 *
 * Named breakpoints (use these in child classNames):
 *   @min-[600px]/surface:   — "sm" equivalent (narrow panels, mobile-ish)
 *   @min-[900px]/surface:   — "md" — sidebar+content layouts activate
 *   @min-[1200px]/surface:  — "lg" — multi-column grids, side-by-side panels
 *
 * Usage:
 *   <AdaptiveSurface>
 *     <div className="grid grid-cols-1 @min-[900px]/surface:grid-cols-[240px_1fr]">
 *       ...
 *     </div>
 *   </AdaptiveSurface>
 */
export function AdaptiveSurface({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("@container/surface h-full min-h-0", className)}>
      {children}
    </div>
  );
}
