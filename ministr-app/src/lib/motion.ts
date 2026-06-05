/**
 * Motion system — principled timing derived from UX intent.
 *
 * Philosophy: "Communicate, don't decorate." Every animation answers one of:
 *   1. Where did this come from / go to? → spring
 *   2. What content just changed?        → flow
 *   3. Was my action received?           → swift
 *   4. Is a value still resolving?       → springSoft
 * If none applies, don't animate.
 *
 * Reduced-motion: gated via `<MotionConfig reducedMotion="user">` at the
 * app root. Individual call sites don't re-check — the provider handles it.
 *
 * See DESIGN.md "Motion system" for the full derivation + decision tree.
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

/* ---- Transitions (see DESIGN.md § Motion system for derivation) ---- */

/** UX role: acknowledgement. "Was my action received?"
 *  140ms — faster than Apple HIG's 200ms because keyboard-first tools
 *  need instant feedback. Material "standard" easing (decelerate-dominant). */
export const swift: Transition = { duration: 0.14, ease: [0.4, 0, 0.2, 1] };

/** UX role: content transition. "What just changed?"
 *  240ms — enough to orient but never sluggish. Strong deceleration
 *  (fast entry, gentle settle) matches Apple HIG macOS panel transitions. */
export const flow: Transition = { duration: 0.24, ease: [0.22, 1, 0.36, 1] };

/** UX role: spatial movement. "Where did this come from / go to?"
 *  Critically-damped spring (ζ≈0.93, no overshoot). Reaches target ~180ms.
 *  Physicality without bounce — professional, not playful. */
export const spring: Transition = {
  type: "spring",
  stiffness: 420,
  damping: 36,
  mass: 0.9,
};

/** UX role: value resolution. "Is this still computing?"
 *  Softer spring (ζ≈1.03 at mass=1). Slower arrival ~300ms communicates
 *  ongoing computation without bouncing. */
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

/** Boot / splash choreography — a calm staggered reveal for the launch hero.
 *  Pair the container with `bootMedallion` (the brand mark) + `bootRise` (the
 *  wordmark / status rows). Settles to the visible final frame, so it's safe
 *  under reduced motion (the MotionProvider snaps it) and for axe snapshots. */
export const bootReveal: Variants = {
  animate: { transition: { staggerChildren: 0.12, delayChildren: 0.04 } },
};

/** The brand medallion's entrance — scale + fade on the soft spring, echoing
 *  the "still starting up" intent (value-resolving feel, no bounce). */
export const bootMedallion: Variants = {
  initial: { opacity: 0, scale: 0.82 },
  animate: { opacity: 1, scale: 1, transition: springSoft },
};

/** A boot text row — fade + gentle rise, one beat behind the mark. */
export const bootRise: Variants = {
  initial: { opacity: 0, y: 8 },
  animate: { opacity: 1, y: 0, transition: flow },
};

/** Shared layout id helper — keeps shared-element ids namespaced. */
export const layoutId = (kind: string, id: string) => `le:${kind}:${id}`;
