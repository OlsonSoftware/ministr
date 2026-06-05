import { useEffect, useRef, useState } from "react";

/**
 * Accumulate a scalar into a rolling, capped time-series — one sample per
 * "tick". A tick is any change to `tick` (defaults to the value itself).
 *
 * Pass the polled source object (e.g. the daemon status) as `tick` so a sample
 * is appended exactly ONCE per poll even when the value is unchanged — a flat
 * metric then still scrolls a flat line rather than collapsing to one point.
 * The `tick` guard also makes the append idempotent under React StrictMode's
 * double-invoked effects (the second run sees the same tick and no-ops).
 *
 * Frontend-only: no backend, no timers of its own — it rides whatever cadence
 * the caller already polls at.
 */
export function useRollingSeries(
  value: number | null | undefined,
  cap = 48,
  tick?: unknown,
): number[] {
  const [series, setSeries] = useState<number[]>([]);
  const key = tick ?? value;
  const lastKey = useRef<unknown>(Symbol("init"));

  useEffect(() => {
    if (key === lastKey.current) return;
    lastKey.current = key;
    if (value == null || !Number.isFinite(value)) return;
    setSeries((s) => {
      const next = [...s, value];
      return next.length > cap ? next.slice(next.length - cap) : next;
    });
  }, [key, value, cap]);

  return series;
}
