/**
 * Custom brutalist sidebar icons.
 *
 * Each icon is a 24×24 inline SVG with `fill: currentColor` so it picks
 * up `text-text` / `[var(--color-accent-fg-on)]` from its parent
 * automatically when the rail item is active vs. idle.
 *
 * Solid blocky shapes preferred. 3px strokes where outlined.
 * No transitions, no animations — brutalist.
 */

type IconProps = {
  className?: string;
  /** Accepted for API parity with lucide-react icons; unused here. */
  strokeWidth?: number;
};

function Svg({
  className,
  children,
}: {
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <svg
      viewBox="0 0 24 24"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
      aria-hidden="true"
      focusable="false"
    >
      {children}
    </svg>
  );
}

/** Search — square frame + small angled handle bottom-right. */
export function BrutalSearch({ className }: IconProps) {
  return (
    <Svg className={className}>
      <rect
        x="2"
        y="2"
        width="14"
        height="14"
        fill="none"
        stroke="currentColor"
        strokeWidth="3"
      />
      <rect
        x="14"
        y="14"
        width="9"
        height="3"
        fill="currentColor"
        transform="rotate(45 15 15)"
      />
    </Svg>
  );
}

/** Explore — search frame plus a stacked 3-bar pivot below; signals the
 *  three modes (sections / symbols / bridges) layered under one search. */
export function BrutalExplore({ className }: IconProps) {
  return (
    <Svg className={className}>
      <rect
        x="2"
        y="2"
        width="11"
        height="11"
        fill="none"
        stroke="currentColor"
        strokeWidth="3"
      />
      <rect
        x="11"
        y="11"
        width="7"
        height="3"
        fill="currentColor"
        transform="rotate(45 12 12)"
      />
      <rect x="3" y="16" width="18" height="2" fill="currentColor" />
      <rect x="3" y="20" width="13" height="2" fill="currentColor" />
    </Svg>
  );
}

/** Symbols — 2×2 grid of filled squares (representing kinds: fn, struct, trait, enum). */
export function BrutalSymbols({ className }: IconProps) {
  return (
    <Svg className={className}>
      <rect x="3" y="3" width="8" height="8" fill="currentColor" />
      <rect x="13" y="3" width="8" height="8" fill="currentColor" />
      <rect x="3" y="13" width="8" height="8" fill="currentColor" />
      <rect x="13" y="13" width="8" height="8" fill="currentColor" />
    </Svg>
  );
}

/** Bridge — two filled squares connected by a thick horizontal bar. */
export function BrutalBridge({ className }: IconProps) {
  return (
    <Svg className={className}>
      <rect x="2" y="9" width="6" height="6" fill="currentColor" />
      <rect x="16" y="9" width="6" height="6" fill="currentColor" />
      <rect x="8" y="11" width="8" height="2" fill="currentColor" />
    </Svg>
  );
}

/** Projects — solid folder shape (rectangle + tab on top-left). */
export function BrutalProjects({ className }: IconProps) {
  return (
    <Svg className={className}>
      <path
        d="M2 6 H9 L11 8 H22 V20 H2 Z"
        fill="currentColor"
      />
    </Svg>
  );
}

/** Structure — three nested rectangles (treemap-like). */
export function BrutalStructure({ className }: IconProps) {
  return (
    <Svg className={className}>
      <rect
        x="2"
        y="2"
        width="20"
        height="20"
        fill="none"
        stroke="currentColor"
        strokeWidth="3"
      />
      <rect
        x="6"
        y="6"
        width="12"
        height="12"
        fill="none"
        stroke="currentColor"
        strokeWidth="3"
      />
      <rect x="10" y="10" width="4" height="4" fill="currentColor" />
    </Svg>
  );
}

/** Sessions — three stacked horizontal bars of decreasing width. */
export function BrutalSessions({ className }: IconProps) {
  return (
    <Svg className={className}>
      <rect x="3" y="5" width="18" height="3" fill="currentColor" />
      <rect x="3" y="11" width="14" height="3" fill="currentColor" />
      <rect x="3" y="17" width="10" height="3" fill="currentColor" />
    </Svg>
  );
}

/** Logs — four horizontal lines stacked (like log entries). */
export function BrutalLogs({ className }: IconProps) {
  return (
    <Svg className={className}>
      <rect x="3" y="4" width="18" height="2" fill="currentColor" />
      <rect x="3" y="9" width="13" height="2" fill="currentColor" />
      <rect x="3" y="14" width="18" height="2" fill="currentColor" />
      <rect x="3" y="19" width="9" height="2" fill="currentColor" />
    </Svg>
  );
}

/** Ask — a chevron prompt sigil over a horizontal answer bar. */
export function BrutalAsk({ className }: IconProps) {
  return (
    <Svg className={className}>
      <path
        d="M3 5 L9 11 L3 17"
        fill="none"
        stroke="currentColor"
        strokeWidth="3"
        strokeLinecap="square"
        strokeLinejoin="miter"
      />
      <rect x="11" y="15" width="10" height="3" fill="currentColor" />
    </Svg>
  );
}

/** Settings — hexagonal outline + solid centered dot. */
export function BrutalSettings({ className }: IconProps) {
  return (
    <Svg className={className}>
      <path
        d="M12 2 L21 7 V17 L12 22 L3 17 V7 Z"
        fill="none"
        stroke="currentColor"
        strokeWidth="3"
      />
      <circle cx="12" cy="12" r="3" fill="currentColor" />
    </Svg>
  );
}

/** Pin — push-pin glyph for "pin source to pane". Solid head, vertical stem. */
export function BrutalPin({ className }: IconProps) {
  return (
    <Svg className={className}>
      <rect x="7" y="2" width="10" height="8" fill="currentColor" />
      <rect x="11" y="10" width="2" height="9" fill="currentColor" />
      <rect x="5" y="19" width="14" height="3" fill="currentColor" />
    </Svg>
  );
}

/** Drawer — three stacked horizontal bars suggesting a pull-out tray. */
export function BrutalDrawer({ className }: IconProps) {
  return (
    <Svg className={className}>
      <rect
        x="2"
        y="3"
        width="20"
        height="18"
        fill="none"
        stroke="currentColor"
        strokeWidth="3"
      />
      <line
        x1="2"
        y1="9"
        x2="22"
        y2="9"
        stroke="currentColor"
        strokeWidth="3"
      />
      <line
        x1="2"
        y1="15"
        x2="22"
        y2="15"
        stroke="currentColor"
        strokeWidth="3"
      />
    </Svg>
  );
}

/** Close — the X glyph with brutal square caps. */
export function BrutalClose({ className }: IconProps) {
  return (
    <Svg className={className}>
      <line
        x1="4"
        y1="4"
        x2="20"
        y2="20"
        stroke="currentColor"
        strokeWidth="3"
        strokeLinecap="square"
      />
      <line
        x1="20"
        y1="4"
        x2="4"
        y2="20"
        stroke="currentColor"
        strokeWidth="3"
        strokeLinecap="square"
      />
    </Svg>
  );
}

/** New — plus glyph for "new investigation" / "new tab". */
export function BrutalNew({ className }: IconProps) {
  return (
    <Svg className={className}>
      <line
        x1="12"
        y1="3"
        x2="12"
        y2="21"
        stroke="currentColor"
        strokeWidth="3"
        strokeLinecap="square"
      />
      <line
        x1="3"
        y1="12"
        x2="21"
        y2="12"
        stroke="currentColor"
        strokeWidth="3"
        strokeLinecap="square"
      />
    </Svg>
  );
}

/** Workspace — three-pane glyph for the "open workspace" idea. */
export function BrutalWorkspace({ className }: IconProps) {
  return (
    <Svg className={className}>
      <rect
        x="2"
        y="3"
        width="20"
        height="18"
        fill="none"
        stroke="currentColor"
        strokeWidth="3"
      />
      <line
        x1="8"
        y1="3"
        x2="8"
        y2="21"
        stroke="currentColor"
        strokeWidth="3"
      />
      <line
        x1="16"
        y1="3"
        x2="16"
        y2="21"
        stroke="currentColor"
        strokeWidth="3"
      />
    </Svg>
  );
}
