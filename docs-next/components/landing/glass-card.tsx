import type { ReactNode } from 'react';

/**
 * GlassCard — labelled panel surface. Styling lives in
 * `global.css` (`.glass-card`); the component just applies the class
 * so the surface treatment stays consistent wherever it's used.
 *
 * Historical note: this used to be a frosted-glass panel with a
 * rainbow gradient halo and a white sheen gradient overlay — the
 * whole glassmorphism slop package. It's now a flat tinted surface
 * with a plain border; the `.glass-card` class name is retained to
 * avoid churning every call-site for a cosmetic refactor.
 */
export function GlassCard({
  children,
  className = '',
  padded = true,
}: {
  children: ReactNode;
  className?: string;
  padded?: boolean;
}) {
  return (
    <div className={'glass-card ' + (padded ? 'p-5 sm:p-6 ' : '') + className}>
      {children}
    </div>
  );
}
