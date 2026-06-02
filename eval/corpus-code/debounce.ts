/**
 * Rate-control utilities for high-frequency event streams (keystrokes, scroll,
 * resize). Part of the code-heavy evaluation corpus (eval/corpus-code) used to
 * benchmark embedders on natural-language-to-code retrieval.
 */

export type AnyFn = (...args: unknown[]) => void;

/**
 * Delay invoking `fn` until `waitMs` has elapsed since the last call.
 *
 * Every new call cancels the pending timer and starts a fresh one, so a rapid
 * burst of events collapses into a single trailing invocation. Useful for
 * deferring expensive work (a search request, a layout pass) until the user
 * stops typing.
 */
export function debounce<F extends AnyFn>(fn: F, waitMs: number): F {
  let timer: ReturnType<typeof setTimeout> | undefined;
  return ((...args: unknown[]) => {
    if (timer !== undefined) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = undefined;
      fn(...args);
    }, waitMs);
  }) as F;
}

/**
 * Allow `fn` to run at most once per `intervalMs`, ignoring calls in between.
 *
 * Unlike debounce, throttle guarantees a steady cadence of invocations during a
 * sustained stream rather than waiting for it to go quiet — the leading call
 * fires immediately and subsequent ones are dropped until the window reopens.
 */
export function throttle<F extends AnyFn>(fn: F, intervalMs: number): F {
  let last = 0;
  return ((...args: unknown[]) => {
    const now = Date.now();
    if (now - last >= intervalMs) {
      last = now;
      fn(...args);
    }
  }) as F;
}

/**
 * Wrap an async function so overlapping calls share one in-flight promise.
 *
 * While a call is pending, additional callers receive the same promise instead
 * of triggering duplicate work; once it settles the next call starts afresh.
 * This deduplicates concurrent requests for the same resource.
 */
export function coalesce<T>(fn: () => Promise<T>): () => Promise<T> {
  let inFlight: Promise<T> | undefined;
  return () => {
    if (inFlight === undefined) {
      inFlight = fn().finally(() => {
        inFlight = undefined;
      });
    }
    return inFlight;
  };
}
