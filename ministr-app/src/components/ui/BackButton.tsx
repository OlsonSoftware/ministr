import { ActionChip } from "./ActionChip";

/**
 * BackButton — labeled wayfinding (gui-ux-wayfinding-feed-access). A bare
 * "‹" glyph is not legible navigation; pairing the chevron with the
 * destination name ("‹ All projects") tells a first-timer exactly where
 * back goes (Nielsen #1 visibility + #3 a clearly-marked exit). Calm
 * furniture: it is just a quiet ActionChip, no second hue.
 */
export function BackButton({
  onClick,
  label,
}: {
  onClick: () => void;
  /** The destination this returns to, e.g. "All projects". */
  label: string;
}) {
  return (
    <ActionChip onClick={onClick} aria-label={`back to ${label}`}>
      <span aria-hidden className="mr-1">
        ‹
      </span>
      {label}
    </ActionChip>
  );
}
