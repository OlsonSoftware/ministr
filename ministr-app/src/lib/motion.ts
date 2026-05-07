/**
 * Refined-brutalist motion constants.
 *
 * Mirrors the CSS variables in App.css. Use these from JS when you need
 * to drive timeouts/animation durations (e.g. closing a drawer after a
 * slide-out, or matching a fade-resolve to a phase event arrival).
 *
 * Rule: motion exists ONLY where data is moving. Don't import these for
 * hover/active/select states — those stay snap.
 */

export const MOTION_DATA_DURATION_MS = 200;

export const MOTION_DATA_EASING = "cubic-bezier(0.2, 0.8, 0.2, 1)";

/**
 * Inline-style transition for components that can't use the `motion-data`
 * utility class (e.g. dynamic transition-property values).
 */
export const motionDataStyle = (property: string): React.CSSProperties => ({
  transitionProperty: property,
  transitionDuration: `${MOTION_DATA_DURATION_MS}ms`,
  transitionTimingFunction: MOTION_DATA_EASING,
});

/**
 * Returns true if the user has requested reduced motion. Consult before
 * starting any non-essential animation.
 */
export function prefersReducedMotion(): boolean {
  if (typeof window === "undefined") return false;
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}
