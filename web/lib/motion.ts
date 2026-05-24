/**
 * Shared motion/react easings.
 *
 * motion v12 tightened `Transition.ease` to `Easing | Easing[]`, where an
 * inline cubic-bezier literal like `[0.2, 0.8, 0.2, 1]` infers as `number[]`
 * and no longer satisfies the tuple form. Exporting an explicitly-typed
 * tuple keeps every call-site clean and keeps a single source of truth for
 * the landing's easing curve.
 */
export const EASE_OUT: [number, number, number, number] = [0.2, 0.8, 0.2, 1];
