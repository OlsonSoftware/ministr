import type { CorpusInfo, IndexingStatus } from "./types";

/** Visual tone shared across StatusDot, Badge, MetricTile, and the
 *  budget/pressure indicators. The five tones map to the design-token
 *  CSS variables (`--color-success`, `--color-warning`, `--color-danger`,
 *  `--color-accent`, `--color-text-dim`) defined in App.css. */
export type Tone = "success" | "warning" | "danger" | "accent" | "muted";

/** Badge's `variant` table happens to omit `accent` because its
 *  `default` variant already paints in accent colors. Use
 *  `toneToBadgeVariant` when you have a Tone and need a Badge variant. */
export type BadgeVariant = "default" | "success" | "warning" | "danger" | "muted";

const INDEXING_TONE: Record<IndexingStatus["state"], Tone> = {
  idle: "success",
  indexing: "warning",
  error: "danger",
};

const INDEXING_LABEL: Record<IndexingStatus["state"], string> = {
  idle: "Ready",
  indexing: "Indexing",
  error: "Error",
};

const PRESSURE_TONE: Record<string, Tone> = {
  none: "muted",
  low: "success",
  medium: "accent",
  high: "warning",
  critical: "danger",
};

const TONE_TEXT: Record<Tone, string> = {
  success: "text-success",
  warning: "text-warning",
  danger: "text-danger",
  accent: "text-accent",
  muted: "text-text-dim",
};

const TONE_BG: Record<Tone, string> = {
  success: "bg-success",
  warning: "bg-warning",
  danger: "bg-danger",
  accent: "bg-accent",
  muted: "bg-text-dim",
};

/** Tone for an IndexingStatus alone (no session activity). */
export function indexingTone(status: IndexingStatus): Tone {
  return INDEXING_TONE[status.state];
}

/** Short user-facing label for an IndexingStatus. */
export function indexingLabel(status: IndexingStatus): string {
  return INDEXING_LABEL[status.state];
}

/** Tone for a corpus, considering both indexing state and whether
 *  sessions are attached. `accent` means idle-but-live
 *  (sessions > 0); use this for chip selectors and rollup dots. */
export function corpusTone(corpus: CorpusInfo): Tone {
  if (corpus.status.state === "error") return "danger";
  if (corpus.status.state === "indexing") return "warning";
  if (corpus.active_sessions > 0) return "accent";
  return "muted";
}

/** A corpus is "live" if it's actively indexing or has active
 *  sessions. Drives whether StatusDot should pulse. */
export function isCorpusLive(corpus: CorpusInfo): boolean {
  return corpus.status.state === "indexing" || corpus.active_sessions > 0;
}

/** Tone for a session pressure level. Falls back to `muted` for
 *  unknown values (forward-compatible with future daemon additions). */
export function pressureTone(pressure: string): Tone {
  return PRESSURE_TONE[pressure] ?? "muted";
}

/** Tailwind text-color class for a tone. */
export function toneTextClass(tone: Tone): string {
  return TONE_TEXT[tone];
}

/** Tailwind background-color class for a tone. */
export function toneBgClass(tone: Tone): string {
  return TONE_BG[tone];
}

/** Map a Tone to the closest Badge variant. `accent` maps to
 *  `default` (Badge's default variant paints in accent colors). */
export function toneToBadgeVariant(tone: Tone): BadgeVariant {
  return tone === "accent" ? "default" : tone;
}

/** One-call helper for the common case: render a status as a Badge.
 *  Returns the variant + label; the caller composes the JSX so it
 *  can choose to add a `dot` prop or extra classes. */
export function statusBadge(
  status: IndexingStatus,
): { variant: BadgeVariant; label: string } {
  return {
    variant: toneToBadgeVariant(indexingTone(status)),
    label: indexingLabel(status),
  };
}
