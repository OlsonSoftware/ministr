'use client';

import type { ReactNode } from 'react';
import { motion, useReducedMotion } from 'motion/react';

/**
 * Reveal — animated scroll-in wrapper.
 *
 * Fades + lifts children as they enter the viewport. Uses a generous
 * viewport margin and `amount: 0` so the reveal fires the moment any
 * pixel of the element enters the viewport — keeps behaviour robust
 * under Playwright fullPage captures and lazy viewports.
 *
 * Respects prefers-reduced-motion by going static.
 */
export function Reveal({
  children,
  delay = 0,
  y = 14,
  className = '',
  as = 'div',
}: {
  children: ReactNode;
  delay?: number;
  y?: number;
  className?: string;
  as?: 'div' | 'section' | 'h2' | 'h3' | 'p' | 'li';
}) {
  const reduced = useReducedMotion();
  const Comp = motion[as] as typeof motion.div;
  // Start at 0.35 (not 0) so content is always legible even if the
  // IntersectionObserver fires after a fast scroll; the lift animation
  // still reads clearly when it completes.
  return (
    <Comp
      initial={reduced ? false : { opacity: 0.35, y }}
      whileInView={reduced ? undefined : { opacity: 1, y: 0 }}
      viewport={{ once: true, amount: 0, margin: '200px 0px 200px 0px' }}
      transition={{ duration: 0.55, ease: [0.2, 0.8, 0.2, 1], delay }}
      className={className}
    >
      {children}
    </Comp>
  );
}
