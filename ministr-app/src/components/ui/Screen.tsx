import type { ReactNode } from "react";
import { LiveDot } from "./LiveDot";

/**
 * Screen — the shared shell every top-level view composes onto (DESIGN.md
 * v5, UX-BLUEPRINT v4.1). It owns the one vertical rhythm the four screen
 * roots used to hand-roll (and had drifted on): a bounded column with an
 * optional header slot, a content region that CENTERS when short and
 * SCROLLS when tall, and a persistent thin NEUTRAL trust-footer.
 *
 * The footer is the calm "mission control" baseline — presence + locality
 * + version, neutrals only (the lone amber is LiveDot's sanctioned §7
 * presence dot). Nothing here glows, blurs, or out-shouts the content.
 *
 * Centering uses `m-auto` on the inner content wrapper rather than
 * `justify-center` on the scroll container: auto margins collapse cleanly
 * once content overflows, so tall content scrolls from the top instead of
 * being clipped — center-when-short AND scroll-when-tall from one rule.
 */

type Width = "xl" | "2xl" | "3xl";
type Gap = "sm" | "md" | "lg";
type Align = "start" | "center";

const WIDTH: Record<Width, string> = {
  xl: "max-w-xl",
  "2xl": "max-w-2xl",
  "3xl": "max-w-3xl",
};

const GAP: Record<Gap, string> = {
  sm: "gap-3",
  md: "gap-4",
  lg: "gap-6",
};

export function Screen({
  header,
  children,
  footer,
  align = "start",
  width = "3xl",
  gap = "md",
  version,
}: {
  /** Optional header slot (Brand + controls, a back affordance, etc.). */
  header?: ReactNode;
  children: ReactNode;
  /** Override the default trust-footer. Pass `null` to omit it entirely. */
  footer?: ReactNode | null;
  /** Vertical placement of content when it is shorter than the viewport. */
  align?: Align;
  /** Max content column width. */
  width?: Width;
  /** Vertical rhythm between content children. */
  gap?: Gap;
  /** App version surfaced in the default trust-footer (e.g. "0.6.0"). */
  version?: string;
}) {
  return (
    <div
      className={`mx-auto flex h-screen w-full ${WIDTH[width]} flex-col gap-4 overflow-hidden p-8`}
    >
      {header ? <header className="shrink-0">{header}</header> : null}

      {/* tabIndex makes the scroll region keyboard-reachable when its
          content isn't itself focusable (axe scrollable-region-focusable). */}
      <main tabIndex={0} className="flex min-h-0 flex-1 flex-col overflow-y-auto">
        <div
          className={`flex w-full flex-col ${GAP[gap]} ${
            align === "center" ? "m-auto" : ""
          }`}
        >
          {children}
        </div>
      </main>

      {footer === null ? null : (
        <footer className="shrink-0">
          {footer ?? <TrustFooter version={version} />}
        </footer>
      )}
    </div>
  );
}

/** The default baseline: presence · locality · version, neutrals only. */
function TrustFooter({ version }: { version?: string }) {
  return (
    <div className="flex items-center justify-between border-t border-line pt-3 text-sm text-dim">
      <LiveDot label="ministr running" />
      <span>{version ? `all local · v${version}` : "all local"}</span>
    </div>
  );
}
