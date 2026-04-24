/**
 * AuroraBackdrop — layered OKLCH radial gradients behind a section.
 *
 * 2026 mesh/aurora aesthetic (Apple/Stripe/Linear lineage): pure CSS,
 * GPU-composited, respects prefers-reduced-motion. Keep it absolute +
 * pointer-events-none so content sits on top without wrestling layout.
 */
export function AuroraBackdrop({
  animated = true,
  scrim = true,
  className = '',
}: {
  animated?: boolean;
  scrim?: boolean;
  className?: string;
}) {
  return (
    <div aria-hidden className={'pointer-events-none absolute inset-0 overflow-hidden ' + className}>
      <div className={'aurora-layer' + (animated ? ' aurora-layer--drift' : '')} />
      {scrim && <div className="aurora-scrim" />}
    </div>
  );
}
