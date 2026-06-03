import type React from "react";
import { cn } from "../../lib/utils";
import {
  headingChapter,
  headingDisplay,
  labelSmallCap,
} from "../../lib/ui-tokens";

/**
 * Page title — the `headingDisplay` token (Geist sans, ~2xl, semibold,
 * tight per DESIGN.md §6). Use as the topmost h1 of a tab/screen body.
 * The screen description below should be a `<p>` with `marginalia` or
 * `bodyMuted` styling — defined per callsite.
 */
interface HeadingProps {
  className?: string;
  children: React.ReactNode;
}

export function H1({ className, children }: HeadingProps) {
  return <h1 className={cn(headingDisplay, className)}>{children}</h1>;
}

/**
 * Section / chapter heading — the `headingChapter` token (Geist sans,
 * base, semibold, snug per §6). Used inside EntityPanel sections, Settings
 * groups, Onboarding step pages, Empty-state titles. Pairs naturally with
 * a `chapterIndex` `§N` marker rendered alongside.
 */
export function H2({ className, children }: HeadingProps) {
  return <h2 className={cn(headingChapter, className)}>{children}</h2>;
}

/**
 * Sub-section / data-zone label (mono uppercase tracked, ~12px).
 * Use as the header inside compact data panels, kind summary blocks,
 * filter strip captions.
 */
export function H3({ className, children }: HeadingProps) {
  return <h3 className={cn(labelSmallCap, className)}>{children}</h3>;
}
