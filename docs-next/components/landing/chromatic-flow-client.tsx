'use client';

import dynamic from 'next/dynamic';
import { useEffect, useState } from 'react';

/**
 * Client-only wrapper for the ChromaticFlow shader.
 *
 * Defers the WebGL work off the critical path:
 * - Only imports + mounts after first-paint is idle (requestIdleCallback
 *   with setTimeout fallback).
 * - Skips entirely on narrow viewports (< 768px) — mobile devices pay
 *   the highest GPU-vs-battery cost and visual payoff is lowest.
 * - Skips when the user prefers reduced motion.
 *
 * Keeps hero TTI fast; the shader fades in later and nobody notices.
 */
const ChromaticFlow = dynamic(
  () => import('@/components/landing/chromatic-flow').then((m) => m.ChromaticFlow),
  { ssr: false, loading: () => null },
);

export function ChromaticFlowClient() {
  const [shouldMount, setShouldMount] = useState(false);

  useEffect(() => {
    // Mobile: bail on the shader entirely. Pointer-coarse devices read
    // the backdrop as noise and pay the full GPU cost.
    if (typeof window === 'undefined') return;
    if (window.matchMedia('(max-width: 767px)').matches) return;

    // Reduced-motion users don't get the shader either — the shader's
    // whole job is motion-driven ambience.
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;

    // Defer the WebGL load until the browser is idle. Safari + older
    // Firefox lack requestIdleCallback; fall back to a short setTimeout
    // that still clears the initial paint + hero-render frames.
    const idle =
      (window as unknown as { requestIdleCallback?: (cb: () => void, opts?: { timeout: number }) => number })
        .requestIdleCallback;
    let handle: number;
    if (typeof idle === 'function') {
      handle = idle(() => setShouldMount(true), { timeout: 2500 });
    } else {
      handle = window.setTimeout(() => setShouldMount(true), 600);
    }
    return () => {
      const cancel =
        (window as unknown as { cancelIdleCallback?: (h: number) => void }).cancelIdleCallback;
      if (typeof cancel === 'function') cancel(handle);
      else window.clearTimeout(handle);
    };
  }, []);

  if (!shouldMount) return null;
  return <ChromaticFlow />;
}
