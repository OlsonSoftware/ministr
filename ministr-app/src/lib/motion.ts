/**
 * Cockpit motion system.
 *
 * Motion is a first-class part of the language now (the old "motion only
 * where data moves" rule is retired). Everything here is gated behind the
 * OS reduced-motion setting via `<MotionProvider>` (MotionConfig
 * reducedMotion="user"), so individual call sites don't each re-check.
 *
 * Mirrors the easing/duration tokens in App.css.
 */
import type { Transition, Variants } from "motion/react";

/* ---- Legacy aliases (kept so unmigrated callers keep compiling) ---- */
export const MOTION_DATA_DURATION_MS = 240;
export const MOTION_DATA_EASING = "cubic-bezier(0.22, 1, 0.36, 1)";

export const motionDataStyle = (property: string): React.CSSProperties => ({
  transitionProperty: property,
  transitionDuration: `${MOTION_DATA_DURATION_MS}ms`,
  transitionTimingFunction: MOTION_DATA_EASING,
});

export function prefersReducedMotion(): boolean {
  if (typeof window === "undefined") return false;
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/* ---- Transitions ---- */

/** Snappy — chrome, hover, nav indicator, tab underline. */
export const swift: Transition = { duration: 0.14, ease: [0.4, 0, 0.2, 1] };

/** Smooth — surface/page transitions, fades. */
export const flow: Transition = { duration: 0.24, ease: [0.22, 1, 0.36, 1] };

/** Springy — panels, inspectors, shared-element layout transitions. */
export const spring: Transition = {
  type: "spring",
  stiffness: 420,
  damping: 36,
  mass: 0.9,
};

/** Soft spring — number tickers, gentle value changes. */
export const springSoft: Transition = {
  type: "spring",
  stiffness: 210,
  damping: 30,
};

/* ---- Variants ---- */

/** Fade + small rise. The workhorse enter/exit. */
export const fadeRise: Variants = {
  initial: { opacity: 0, y: 6 },
  animate: { opacity: 1, y: 0, transition: flow },
  exit: { opacity: 0, y: -4, transition: swift },
};

/** Fade only — for cross-surface swaps where movement would distract. */
export const fade: Variants = {
  initial: { opacity: 0 },
  animate: { opacity: 1, transition: flow },
  exit: { opacity: 0, transition: swift },
};

/** Inspector slide-over (from the right). */
export const slideOver: Variants = {
  initial: { x: "100%" },
  animate: { x: 0, transition: spring },
  exit: { x: "100%", transition: { ...spring, damping: 40 } },
};

/** Modal / palette pop. */
export const popIn: Variants = {
  initial: { opacity: 0, scale: 0.97, y: -8 },
  animate: { opacity: 1, scale: 1, y: 0, transition: spring },
  exit: { opacity: 0, scale: 0.98, y: -4, transition: swift },
};

/** Backdrop scrim. */
export const scrim: Variants = {
  initial: { opacity: 0 },
  animate: { opacity: 1, transition: flow },
  exit: { opacity: 0, transition: swift },
};

/** Staggered list container — pair with `listItem` on children. */
export const listContainer: Variants = {
  animate: { transition: { staggerChildren: 0.035, delayChildren: 0.02 } },
};

export const listItem: Variants = {
  initial: { opacity: 0, y: 8 },
  animate: { opacity: 1, y: 0, transition: flow },
  exit: { opacity: 0, y: -4, transition: swift },
};

/** Shared layout id helper — keeps shared-element ids namespaced. */
export const layoutId = (kind: string, id: string) => `le:${kind}:${id}`;
