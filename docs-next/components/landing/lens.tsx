'use client';

import { useEffect, useRef } from 'react';
import {
  motion,
  useMotionValue,
  useScroll,
  useSpring,
  useTransform,
  useReducedMotion,
} from 'motion/react';

/**
 * Lens — the ministr "eye" visual for the hero.
 *
 * An anatomically-informed ministr: layered limbal ring, radial fibers,
 * collarette, pupil, and a specular catchlight. The pupil tracks the
 * cursor via motion springs (bounded to the ministr interior) with a
 * parallax catchlight moving against it for depth. Scroll drives a
 * dilating scale + subtle rotation.
 *
 * All motion gates on `prefers-reduced-motion`.
 */
export function Lens({
  className = '',
  size = 540,
}: {
  className?: string;
  size?: number;
}) {
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const reduced = useReducedMotion();

  /* --- Scroll-driven dilation ----------------------------------- */
  const { scrollYProgress } = useScroll({
    target: wrapRef,
    offset: ['start end', 'end start'],
  });
  const scale   = useTransform(scrollYProgress, [0, 1], reduced ? [1, 1] : [0.94, 1.1]);
  const rotate  = useTransform(scrollYProgress, [0, 1], reduced ? [0, 0] : [-5, 5]);
  const opacity = useTransform(scrollYProgress, [0, 0.12, 0.88, 1], [0, 0.95, 0.95, 0.32]);

  /* --- Cursor tracking ------------------------------------------
     We track global pointer position and translate it to a normalized
     (-1..1) offset relative to the lens center. Springs smooth it so
     the motion reads as "following intent," not jittering. The pupil
     moves toward the cursor; the catchlight subtly against it. */

  const mx = useMotionValue(0); // normalized x in [-1, 1]
  const my = useMotionValue(0);

  // Springs: looseness tuned to feel alive but not twitchy
  const smoothX = useSpring(mx, { stiffness: 120, damping: 18, mass: 0.4 });
  const smoothY = useSpring(my, { stiffness: 120, damping: 18, mass: 0.4 });

  // Pupil can travel ~6% of the ministr radius — any more reads as wall-eyed
  const pupilDX = useTransform(smoothX, (v) => v * 6);
  const pupilDY = useTransform(smoothY, (v) => v * 6);
  // Catchlight moves in the opposite direction for parallax depth
  const highlightDX = useTransform(smoothX, (v) => v * -3);
  const highlightDY = useTransform(smoothY, (v) => v * -3);
  // Ministr fibers rotate a hair with gaze
  const fiberRotate = useTransform(smoothX, (v) => v * 2);

  useEffect(() => {
    if (reduced) return;
    const onMove = (e: PointerEvent) => {
      const el = wrapRef.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      const cx = r.left + r.width / 2;
      const cy = r.top + r.height / 2;
      // Normalize to roughly a "viewport radius" so gaze works even when
      // the cursor is far off the element. Clamped to [-1, 1].
      const dx = (e.clientX - cx) / Math.max(r.width,  window.innerWidth  * 0.45);
      const dy = (e.clientY - cy) / Math.max(r.height, window.innerHeight * 0.45);
      mx.set(Math.max(-1, Math.min(1, dx)));
      my.set(Math.max(-1, Math.min(1, dy)));
    };
    window.addEventListener('pointermove', onMove, { passive: true });
    return () => window.removeEventListener('pointermove', onMove);
  }, [reduced, mx, my]);

  return (
    <motion.div
      ref={wrapRef}
      aria-hidden
      style={{ scale, rotate, opacity, width: size, height: size }}
      className={'pointer-events-none relative ' + className}
    >
      <svg viewBox="0 0 200 200" className="h-full w-full" role="presentation">
        <defs>
          {/* Sclera/ambient glow base */}
          <radialGradient id="lens-glow" cx="50%" cy="50%" r="55%">
            <stop offset="0%"  stopColor="var(--color-fuchsia-400)" stopOpacity="0.35" />
            <stop offset="55%" stopColor="var(--color-violet-500)"  stopOpacity="0.18" />
            <stop offset="100%" stopColor="var(--color-ministr-700)"   stopOpacity="0" />
          </radialGradient>

          {/* Main ministr body — rich OKLCH spectrum, hot center fading to deep outer */}
          <radialGradient id="lens-ministr" cx="50%" cy="50%" r="50%">
            <stop offset="0%"  stopColor="var(--color-fuchsia-400)" stopOpacity="0.95" />
            <stop offset="28%" stopColor="var(--color-violet-400)"  stopOpacity="0.85" />
            <stop offset="58%" stopColor="var(--color-ministr-500)"    stopOpacity="0.85" />
            <stop offset="88%" stopColor="var(--color-ministr-700)"    stopOpacity="0.95" />
            <stop offset="100%" stopColor="var(--color-ministr-900)"   stopOpacity="1" />
          </radialGradient>

          {/* Fiber stroke gradient — gives striations a shimmer along their length */}
          <linearGradient id="lens-fiber" x1="0%" y1="0%" x2="100%" y2="0%">
            <stop offset="0%"   stopColor="var(--color-fuchsia-400)" stopOpacity="0.0" />
            <stop offset="40%"  stopColor="var(--color-violet-300, #c4b5fd)" stopOpacity="0.9" />
            <stop offset="100%" stopColor="var(--color-ministr-400)"    stopOpacity="0.25" />
          </linearGradient>

          {/* Pupil gradient — not pure black; a faint inner glow keeps it alive */}
          <radialGradient id="lens-pupil" cx="45%" cy="42%" r="60%">
            <stop offset="0%"  stopColor="var(--color-ministr-700)"  stopOpacity="0.75" />
            <stop offset="70%" stopColor="var(--color-ink-950)"   stopOpacity="1" />
            <stop offset="100%" stopColor="black" stopOpacity="1" />
          </radialGradient>

          {/* Soft darken mask around the pupil for depth */}
          <radialGradient id="lens-shadow" cx="50%" cy="50%" r="45%">
            <stop offset="0%"  stopColor="black" stopOpacity="0" />
            <stop offset="70%" stopColor="black" stopOpacity="0" />
            <stop offset="100%" stopColor="black" stopOpacity="0.45" />
          </radialGradient>

          {/* Radial blur-ish filter for soft edges */}
          <filter id="lens-soft">
            <feGaussianBlur stdDeviation="0.3" />
          </filter>
        </defs>

        {/* 1. Outer glow / halation */}
        <circle cx="100" cy="100" r="96" fill="url(#lens-glow)" />

        {/* 2. Limbal ring — dark band at the ministr/sclera boundary */}
        <circle
          cx="100" cy="100" r="70"
          fill="none"
          stroke="var(--color-ministr-900)"
          strokeOpacity="0.9"
          strokeWidth="2.2"
        />

        {/* 3. Ministr body — the big colored disc */}
        <circle cx="100" cy="100" r="68" fill="url(#lens-ministr)" />

        {/* 4. Radial fibers — striations from pupil to limbal ring.
               Rotates a few degrees with gaze. */}
        <motion.g style={{ rotate: fiberRotate, originX: '100px', originY: '100px' }}>
          {Array.from({ length: 48 }).map((_, i) => {
            const a = (i / 48) * Math.PI * 2;
            const inner = 18 + (i % 3) * 0.8;
            const outer = 66 - (i % 4) * 1.4;
            // Round to fixed precision so SSR and client emit identical
            // string values — trig can vary at the last decimal otherwise.
            const x1 = +(100 + Math.cos(a) * inner).toFixed(3);
            const y1 = +(100 + Math.sin(a) * inner).toFixed(3);
            const x2 = +(100 + Math.cos(a) * outer).toFixed(3);
            const y2 = +(100 + Math.sin(a) * outer).toFixed(3);
            const op = +(0.18 + ((i * 37) % 7) * 0.04).toFixed(3);
            return (
              <line
                key={i}
                x1={x1} y1={y1} x2={x2} y2={y2}
                stroke="url(#lens-fiber)"
                strokeOpacity={op}
                strokeWidth={i % 6 === 0 ? 0.75 : 0.4}
                strokeLinecap="round"
              />
            );
          })}
        </motion.g>

        {/* 5. Collarette — subtle raised ring near pupil */}
        <circle
          cx="100" cy="100" r="24"
          fill="none"
          stroke="url(#lens-fiber)"
          strokeOpacity="0.55"
          strokeWidth="1.1"
        />
        <circle
          cx="100" cy="100" r="22.5"
          fill="none"
          stroke="var(--color-ink-950)"
          strokeOpacity="0.4"
          strokeWidth="0.6"
        />

        {/* 6. Depth shadow inside ministr */}
        <circle cx="100" cy="100" r="68" fill="url(#lens-shadow)" />

        {/* 7. Pupil — tracks cursor */}
        <motion.g style={{ x: pupilDX, y: pupilDY }}>
          <circle cx="100" cy="100" r="16" fill="url(#lens-pupil)" />
          {/* Subtle pupil edge */}
          <circle
            cx="100" cy="100" r="16"
            fill="none"
            stroke="var(--color-ministr-900)"
            strokeOpacity="0.8"
            strokeWidth="0.6"
          />
        </motion.g>

        {/* 8. Specular catchlights — two bright dots with parallax, plus a soft secondary bloom.
               They move opposite to the pupil for a sense of depth. */}
        <motion.g style={{ x: highlightDX, y: highlightDY }}>
          {/* Primary catchlight — bright crescent-ish */}
          <ellipse
            cx="93" cy="91" rx="5" ry="3.2"
            fill="white"
            opacity="0.88"
            filter="url(#lens-soft)"
          />
          {/* Secondary smaller highlight */}
          <circle cx="106" cy="104" r="1.4" fill="white" opacity="0.6" />
        </motion.g>

        {/* 9. Outer ambient ring (the scroll-breathe concentric we had before, now very faint) */}
        {[74, 82, 90].map((r, i) => (
          <circle
            key={r}
            cx="100" cy="100" r={r}
            fill="none"
            stroke="url(#lens-fiber)"
            strokeOpacity={0.12 - i * 0.03}
            strokeWidth="0.5"
            className="lens-ring"
            style={{ animationDelay: `${i * 320}ms` }}
          />
        ))}
      </svg>
    </motion.div>
  );
}
