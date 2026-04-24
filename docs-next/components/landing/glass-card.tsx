import type { ReactNode } from 'react';

/**
 * GlassCard — frosted glass panel with spectrum hairline.
 *
 * Panel styling lives in global.css (.glass-card). The component just
 * slaps the class on a div so the visual is consistent across the
 * hero terminal wrap, bento tiles, install block, and CTA coda.
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
