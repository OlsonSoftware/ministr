import type { DerivedProgress } from "../../lib/progress";
import type { TrustState } from "../ui/trust";
import { TRUST } from "../ui/trust";
import { TechRow } from "../ui/TechRow";
import { IndexingInstrument } from "../ui/IndexingInstrument";

/**
 * ProjectCard (GUI v6, gui-v6-visual-language) — the de-text, data-forward
 * replacement for the prose trust row. The app is a project/INDEX MANAGER:
 * this card reads an index's health at a glance with VISUALS, not sentences.
 *
 *  - status is a colored left rail + a glyph chip (shape AND tone, never
 *    color alone) — pre-attentive, no "Your AI is 2 files behind" sentence;
 *  - the body is a numeric STAT STRIP (files / sections / size / indexed-ago)
 *    in tabular figures — scannable like a dashboard, not read like prose;
 *  - management actions are quiet ICON buttons (reindex / configure / remove)
 *    that surface on hover/focus, not text chips;
 *  - live agent presence is a pulse + count, not a sentence.
 *
 * Calm constraint held: flat, one border, no glow/glass/gradient; color is
 * only the sanctioned trust tones, carried by the rail + status chip.
 */

export interface ProjectCardData {
  name: string;
  status: TrustState;
  files: number;
  sections: number;
  /** Human index size, e.g. "2.4 MB" (optional until wired). */
  size?: string;
  /** Relative last-indexed, e.g. "3m ago". */
  indexedAgo?: string;
  /** Count of files behind your changes (drives the status chip). */
  behind?: number;
  stack?: string[];
  /** Agents actively connected to this index right now (lite presence). */
  agents?: number;
  /** Live indexing progress; when running, the instrument replaces the strip. */
  progress?: DerivedProgress;
}

const RAIL: Record<TrustState, string> = {
  ok: "bg-ok",
  stale: "bg-stale",
  hidden: "bg-hidden",
  updating: "bg-brand",
};

export function ProjectCard({
  data,
  headingLevel = 3,
  onOpen,
  onReindex,
  onConfigure,
  onRemove,
}: {
  data: ProjectCardData;
  /** The name's heading level — 3 in the Home list; 2 when the card is a
   *  page summary under an h1 (keeps the heading order valid). */
  headingLevel?: 2 | 3;
  onOpen?: () => void;
  onReindex?: () => void;
  onConfigure?: () => void;
  onRemove?: () => void;
}) {
  const Heading = `h${headingLevel}` as "h2" | "h3";
  const { name, status, files, sections, size, indexedAgo, behind, stack, agents, progress } =
    data;
  const indexing = status === "updating" && progress?.running;
  const meta = TRUST[status];

  return (
    <div className="group relative flex overflow-hidden rounded-lg border border-line bg-surface">
      {/* status rail — the pre-attentive health cue (tone, not text) */}
      <div className={`w-1.5 shrink-0 ${RAIL[status]}`} aria-hidden />

      {/* whole-card open affordance sits under the content (icon actions
          stop propagation so they stay clickable) */}
      {onOpen ? (
        <button
          type="button"
          aria-label={`manage ${name}`}
          onClick={onOpen}
          className="peer absolute inset-0 z-0 cursor-pointer rounded-lg focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand"
        />
      ) : null}

      <div className="relative z-0 min-w-0 flex-1 space-y-3 p-4 peer-hover:bg-sunken/40">
        {/* header: name + status chip + live + hover actions */}
        <div className="flex items-center gap-2.5">
          <span
            role="img"
            aria-label={meta.word}
            className={`text-sm font-semibold ${meta.tone} ${
              status === "updating" ? "pulse-live" : ""
            }`}
          >
            {meta.glyph}
          </span>
          <Heading className="truncate text-base font-semibold text-ink">{name}</Heading>

          {/* Count/label chips stay NEUTRAL — the status COLOR is carried
              accessibly by the rail + the role=img glyph (a 12px tone-tinted
              label fails WCAG contrast). */}
          {behind && behind > 0 && status === "stale" ? (
            <span className="shrink-0 rounded-full bg-sunken px-2 py-0.5 text-xs font-medium tabular-nums text-dim">
              {behind} behind
            </span>
          ) : null}
          {indexing ? (
            <span className="shrink-0 rounded-full bg-sunken px-2 py-0.5 text-xs font-medium text-dim">
              indexing
            </span>
          ) : null}

          <div className="z-0 ml-auto flex shrink-0 items-center gap-1">
            {agents && agents > 0 ? (
              <span
                className="mr-1 flex items-center gap-1 text-xs tabular-nums text-dim"
                aria-label={`${agents} agent${agents === 1 ? "" : "s"} connected`}
              >
                <span className="pulse-live inline-block size-2 rounded-full bg-brand" aria-hidden />
                {agents}
              </span>
            ) : null}
            {/* actions: quiet, surface on hover/focus, always keyboard-reachable */}
            <div className="flex items-center gap-0.5 opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
              {onReindex ? (
                <IconButton label={`reindex ${name}`} onClick={onReindex}>
                  <ReindexIcon />
                </IconButton>
              ) : null}
              {onConfigure ? (
                <IconButton label={`configure ${name}`} onClick={onConfigure}>
                  <GearIcon />
                </IconButton>
              ) : null}
              {onRemove ? (
                <IconButton label={`remove ${name}`} onClick={onRemove}>
                  <TrashIcon />
                </IconButton>
              ) : null}
            </div>
          </div>
        </div>

        {stack && stack.length > 0 ? <TechRow slugs={stack} /> : null}

        {/* the body: an instrument while indexing, else the numeric stat strip */}
        {indexing && progress ? (
          <IndexingInstrument progress={progress} variant="compact" />
        ) : (
          <div className="flex flex-wrap gap-x-8 gap-y-2">
            <Stat value={fmt(files)} label="files" />
            <Stat value={fmt(sections)} label="sections" />
            {size ? <Stat value={size} label="index" /> : null}
            {indexedAgo ? <Stat value={indexedAgo} label="indexed" /> : null}
          </div>
        )}
      </div>
    </div>
  );
}

/** One dashboard figure: a prominent tabular number over a quiet label. */
function Stat({ value, label }: { value: string; label: string }) {
  return (
    <div className="min-w-0">
      <div className="text-lg font-semibold tabular-nums text-ink">{value}</div>
      <div className="text-xs text-dim">{label}</div>
    </div>
  );
}

function IconButton({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      className="flex size-7 items-center justify-center rounded-md text-dim transition-colors hover:bg-sunken hover:text-ink focus-visible:opacity-100 focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-brand"
    >
      {children}
    </button>
  );
}

/** Integers with thousands separators; tolerant of a not-yet-known count
 *  (a corpus mid-registration may lack file/section totals). */
function fmt(n: number | undefined | null): string {
  return (n ?? 0).toLocaleString("en-US");
}

function ReindexIcon() {
  return (
    <svg viewBox="0 0 16 16" className="size-4" fill="none" stroke="currentColor" strokeWidth="1.4" aria-hidden>
      <path d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9" />
      <path d="M13.5 2.5V5H11" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function GearIcon() {
  return (
    <svg viewBox="0 0 16 16" className="size-4" fill="none" stroke="currentColor" strokeWidth="1.4" aria-hidden>
      <circle cx="8" cy="8" r="2.25" />
      <path d="M8 1.5v1.6M8 12.9v1.6M14.5 8h-1.6M3.1 8H1.5M12.6 3.4l-1.1 1.1M4.5 11.5l-1.1 1.1M12.6 12.6l-1.1-1.1M4.5 4.5 3.4 3.4" />
    </svg>
  );
}

function TrashIcon() {
  return (
    <svg viewBox="0 0 16 16" className="size-4" fill="none" stroke="currentColor" strokeWidth="1.4" aria-hidden>
      <path d="M3 4.5h10M6.5 4.5V3h3v1.5M5 4.5l.5 8h5l.5-8" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}
