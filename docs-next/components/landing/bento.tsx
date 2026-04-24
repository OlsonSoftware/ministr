'use client';

import type { ReactNode, MouseEvent } from 'react';
import { useRef } from 'react';
import { motion, useMotionValue, useSpring, useTransform, useReducedMotion } from 'motion/react';

/**
 * BentoGrid — 12-col CSS grid with generous gutters. Container-queries
 * aware so embedded usage stays responsive to the wrapper, not the
 * viewport.
 */
export function BentoGrid({ children, className = '' }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={
        'grid grid-cols-2 md:grid-cols-6 lg:grid-cols-12 gap-4 sm:gap-5 ' + className
      }
    >
      {children}
    </div>
  );
}

/**
 * BentoTile — one cell of the bento grid. Accepts a responsive `span`
 * object and adds a subtle 3D tilt on pointer-move + a sheen layer
 * tracking the cursor.
 */
export function BentoTile({
  children,
  className = '',
  span = { base: 2, md: 3, lg: 4 },
  tilt = true,
}: {
  children: ReactNode;
  className?: string;
  span?: { base?: number; md?: number; lg?: number; rows?: number; rowsMd?: number; rowsLg?: number };
  tilt?: boolean;
}) {
  const ref = useRef<HTMLDivElement | null>(null);
  const reduced = useReducedMotion();

  const mx = useMotionValue(50);
  const my = useMotionValue(30);
  const rx = useSpring(useTransform(my, [0, 100], [6, -6]), { stiffness: 180, damping: 18 });
  const ry = useSpring(useTransform(mx, [0, 100], [-8, 8]), { stiffness: 180, damping: 18 });

  function onMove(e: MouseEvent<HTMLDivElement>) {
    if (!ref.current || reduced || !tilt) return;
    const r = ref.current.getBoundingClientRect();
    const x = ((e.clientX - r.left) / r.width) * 100;
    const y = ((e.clientY - r.top) / r.height) * 100;
    mx.set(x);
    my.set(y);
    ref.current.style.setProperty('--mx', x.toFixed(1));
    ref.current.style.setProperty('--my', y.toFixed(1));
  }
  function onLeave() {
    mx.set(50);
    my.set(30);
  }

  const motionStyle =
    reduced || !tilt
      ? undefined
      : ({ rotateX: rx, rotateY: ry, transformPerspective: 900 } as Record<string, unknown>);

  return (
    <motion.div
      ref={ref}
      onMouseMove={onMove}
      onMouseLeave={onLeave}
      style={motionStyle as never}
      data-span-base={span.base ?? 2}
      data-span-md={span.md ?? ''}
      data-span-lg={span.lg ?? ''}
      data-rows-md={span.rowsMd ?? ''}
      data-rows-lg={span.rowsLg ?? ''}
      className={
        'bento-tile glass-card group relative overflow-hidden p-5 sm:p-6 ' + className
      }
    >
      <div className="bento-tile__sheen" />
      {/* No h-full — tiles size to content. Grid row align-items:start
          (in CSS) lets each tile hug its content height independently,
          which is the bento look we want. */}
      <div className="relative z-10 flex flex-col">{children}</div>
    </motion.div>
  );
}
