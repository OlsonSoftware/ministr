import { TRUST, type TrustState } from "./trust";

/**
 * TrustMark — one trust state as glyph + tone + accessible word
 * (DESIGN.md §7). The `updating` glyph breathes (motion-safe).
 */
export function TrustMark({
  state,
  className = "",
}: {
  state: TrustState;
  className?: string;
}) {
  const meta = TRUST[state];
  return (
    <span
      role="img"
      aria-label={meta.word}
      className={`inline-block w-4 text-center font-semibold select-none ${meta.tone} ${
        state === "updating" ? "pulse-live" : ""
      } ${className}`}
    >
      {meta.glyph}
    </span>
  );
}
