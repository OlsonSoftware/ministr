import type { CSSProperties } from "react";
import { useId } from "react";
import { cn } from "../../lib/utils";

/**
 * The ministr brand mark — the amber square-frame glyph from `brand/logo.svg`
 * (the canonical website/brand asset). One React component so the mark is used
 * identically everywhere; the gradient + amber hex are the brand's, per
 * `brand/README.md` (gradient #F8AC18 → #FF9900).
 *
 * Two modes:
 *  • `gradient` (default) — the full-colour brand mark, for wordmark lockups on
 *    neutral chrome.
 *  • `mono` (`gradient={false}`) — `currentColor`, so the mark tones with its
 *    container (e.g. a command-deck medallion that goes accent → danger).
 */
interface LogoProps {
  className?: string;
  /** Full brand gradient (default) vs. currentColor mono. */
  gradient?: boolean;
  /** Accessible name. When set the mark is exposed as an img; otherwise hidden. */
  title?: string;
  style?: CSSProperties;
}

export function Logo({ className, gradient = true, title, style }: LogoProps) {
  const gradId = `ministr-logo-${useId()}`;
  return (
    <svg
      viewBox="0 0 926 926"
      className={cn("h-5 w-5", className)}
      style={style}
      role={title ? "img" : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
      focusable="false"
    >
      {title && <title>{title}</title>}
      {gradient && (
        <defs>
          <linearGradient
            id={gradId}
            x1="1098.5"
            y1="-174"
            x2="-173.5"
            y2="1092"
            gradientUnits="userSpaceOnUse"
          >
            <stop stopColor="#F8AC18" />
            <stop offset="1" stopColor="#FF9900" />
          </linearGradient>
        </defs>
      )}
      <path
        fillRule="evenodd"
        clipRule="evenodd"
        d="M926 926H0V0H926V926ZM241 241V685H685V241H241Z"
        fill={gradient ? `url(#${gradId})` : "currentColor"}
      />
    </svg>
  );
}

/**
 * The brand lockup — the mark + the `ministr` wordmark. Used in the top chrome.
 * The mark scales to the text (1.05em) so the lockup stays balanced at any size.
 */
export function Wordmark({ className }: { className?: string }) {
  return (
    <span
      className={cn("inline-flex items-center gap-2 select-none", className)}
    >
      <Logo className="h-[1.05em] w-[1.05em]" title="ministr" />
      <span className="ministr-wordmark">ministr</span>
    </span>
  );
}
