/**
 * NoiseOverlay — fixed film-grain layer via inline SVG turbulence.
 *
 * Render once near the top of the page. At 0.025 opacity with
 * overlay blend it adds texture so big gradient fields stop looking
 * like a phone wallpaper. Styling lives in global.css.
 */
export function NoiseOverlay() {
  return <div aria-hidden className="noise-overlay" />;
}
