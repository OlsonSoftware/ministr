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
  // Queued — waiting on a scheduler permit. Accent (not warning) so it reads as
  // "pending work" distinct from the actively-spinning `indexing` warning tone.
  queued: "accent",
  indexing: "warning",
  error: "danger",
};

const INDEXING_LABEL: Record<IndexingStatus["state"], string> = {
  idle: "Ready",
  queued: "Queued",
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

/** Raw CSS custom property a tone resolves to. For SVG `stroke`/`fill`
 *  where a Tailwind class can't be applied (sparkline, economics bar,
 *  budget ring). Mirrors the `--color-*` tokens in App.css. */
const TONE_CSS_VAR: Record<Tone, string> = {
  success: "var(--color-success)",
  warning: "var(--color-warning)",
  danger: "var(--color-danger)",
  accent: "var(--color-accent)",
  muted: "var(--color-text-dim)",
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

/** Raw `var(--color-*)` string for a tone — for SVG fill/stroke. */
export function toneCssVar(tone: Tone): string {
  return TONE_CSS_VAR[tone];
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

/** True when a corpus has never produced any indexed content. The daemon's
 *  `idle` state means BOTH "fully indexed" AND "not yet started / queued"
 *  (see IndexingStatus), so file/section counts are the only way to tell a
 *  finished corpus from an empty one. */
export function isCorpusUnindexed(corpus: CorpusInfo): boolean {
  return (
    corpus.status.state === "idle" &&
    corpus.files_indexed === 0 &&
    corpus.sections_count === 0
  );
}

/** Corpus-aware status badge. Without this, a never-indexed corpus renders
 *  as "Ready"/green because its daemon status is `idle` — indistinguishable
 *  from a fully-indexed one. Show "Not indexed" (muted) for an idle corpus
 *  with zero indexed content so 0-file corpora never masquerade as done. */
export function corpusStatusBadge(
  corpus: CorpusInfo,
): { variant: BadgeVariant; label: string } {
  // gd6: a registered-but-not-yet-loaded corpus. Shown immediately as
  // "Warming up…" (accent) so it doesn't pop into the list once its index
  // finishes loading in the background.
  if (corpus.warming) {
    return { variant: "default", label: "Warming up…" };
  }
  if (isCorpusUnindexed(corpus)) {
    return { variant: "muted", label: "Not indexed" };
  }
  return statusBadge(corpus.status);
}
