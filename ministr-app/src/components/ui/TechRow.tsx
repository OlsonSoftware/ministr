import { TechIcon } from "./TechIcon";
import { techEntry } from "../../lib/techIcons";

/**
 * TechRow — the detected tech stack as a quiet row of TechIcons
 * (gui-card-tech-icons). Unknown slugs are dropped (no broken box) and the
 * visible set is capped with a "+N" overflow so a polyglot repo can't blow
 * out the card. Decorative-but-labelled (each icon names itself).
 */
export function TechRow({
  slugs,
  max = 6,
}: {
  slugs: string[];
  /** Max icons shown before collapsing the rest into "+N". */
  max?: number;
}) {
  const known = slugs.filter((s) => techEntry(s));
  if (known.length === 0) return null;

  const shown = known.slice(0, max);
  const overflow = known.length - shown.length;

  return (
    <ul aria-label="detected tech stack" className="flex flex-wrap items-center gap-2">
      {shown.map((slug) => (
        <li key={slug} className="inline-flex">
          <TechIcon slug={slug} />
        </li>
      ))}
      {overflow > 0 ? (
        <li className="text-xs text-dim" aria-label={`and ${overflow} more`}>
          +{overflow}
        </li>
      ) : null}
    </ul>
  );
}
