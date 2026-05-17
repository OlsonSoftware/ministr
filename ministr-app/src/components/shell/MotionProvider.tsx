import type { ReactNode } from "react";
import { MotionConfig } from "motion/react";
import { flow } from "../../lib/motion";

/**
 * App-wide motion context.
 *
 * `MotionConfig reducedMotion="user"` makes every animation respect the
 * OS "reduce motion" setting automatically, so call sites don't each
 * re-check `prefersReducedMotion()`. Default transition is the cockpit
 * `flow` curve unless a component overrides it.
 *
 * (We use the standard `motion.*` components rather than
 * `LazyMotion`/`m.*` — this is a desktop app, not a size-constrained
 * web bundle, so the strict-mode footgun isn't worth the few KB.)
 */
export function MotionProvider({ children }: { children: ReactNode }) {
  return (
    <MotionConfig reducedMotion="user" transition={flow}>
      {children}
    </MotionConfig>
  );
}
