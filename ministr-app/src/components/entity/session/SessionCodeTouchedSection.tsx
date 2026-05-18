import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import type { ActivityEvent } from "../../../lib/types";
import {
  type FileBucket,
  summarizeCodeTouched,
} from "../../../lib/session-activity-summary";
import { cn } from "../../../lib/utils";
import { EntitySection, EntitySectionEmpty } from "../EntitySection";
import { Chip, ChipGroup } from "../../ui/chip-group";

interface Props {
  chapter: number;
  events: ActivityEvent[];
  loading: boolean;
  /** Called when a file row is activated — wires through to the
   *  activity-timeline target filter so §1 click filters §2. */
  onFilterFile?: (file: string) => void;
}

const COLLAPSED_LIMIT = 8;

/**
 * §1 Code touched — the code-intelligence story for this session. Pure
 * derivation from the activity stream: files visited, symbols looked
 * at, bridges inspected, references audited. Each file row is a
 * click-through that filters the activity timeline below.
 */
export function SessionCodeTouchedSection({
  chapter,
  events,
  loading,
  onFilterFile,
}: Props) {
  const summary = useMemo(() => summarizeCodeTouched(events), [events]);
  const [showAll, setShowAll] = useState(false);

  // Short numeric summary for the section header right-edge — full
  // breakdown lives in the first body row to avoid header wrap.
  const metaCount = `${summary.files.length} / ${summary.symbols.length}`;

  // Empty / loading states still render under the §-numbered shell so
  // the panel doesn't reflow when activity starts streaming in.
  if (loading && events.length === 0) {
    return (
      <EntitySection chapter={chapter} title="Code touched">
        <EntitySectionEmpty label="Loading activity…" />
      </EntitySection>
    );
  }
  if (summary.files.length === 0 && summary.symbols.length === 0) {
    return (
      <EntitySection chapter={chapter} title="Code touched" empty>
        <EntitySectionEmpty label="No code-navigation activity yet for this session." />
      </EntitySection>
    );
  }

  const visibleFiles = showAll
    ? summary.files
    : summary.files.slice(0, COLLAPSED_LIMIT);
  const hiddenCount = summary.files.length - visibleFiles.length;

  return (
    <EntitySection chapter={chapter} title="Code touched" meta={metaCount}>
      {/* Totals breakdown — full sentence in the body to avoid header wrap */}
      <p className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5 border-b border-border-soft px-3 py-2 font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
        <span className="whitespace-nowrap text-text">
          {summary.files.length} file{summary.files.length === 1 ? "" : "s"}
        </span>
        <span aria-hidden>·</span>
        <span className="whitespace-nowrap text-text">
          {summary.symbols.length} symbol
          {summary.symbols.length === 1 ? "" : "s"}
        </span>
        <span aria-hidden>·</span>
        <span className="whitespace-nowrap text-text">
          {summary.refsChecked} ref{summary.refsChecked === 1 ? "" : "s"}
        </span>
        <span aria-hidden>·</span>
        <span className="whitespace-nowrap text-text">
          {summary.bridgeInspections} bridge
          {summary.bridgeInspections === 1 ? "" : "s"}
        </span>
      </p>

      {/* File rows */}
      {summary.files.length > 0 && (
        <div role="list">
          {visibleFiles.map((f) => (
            <FileRow
              key={f.file}
              bucket={f}
              onClick={onFilterFile ? () => onFilterFile(f.file) : undefined}
            />
          ))}
          {hiddenCount > 0 && (
            <button
              type="button"
              onClick={() => setShowAll(true)}
              className="w-full text-left border-t border-border-soft px-3 py-2 font-mono text-mono-mini uppercase tracking-[0.08em] text-text-muted hover:bg-surface-overlay cursor-pointer transition-colors duration-150"
            >
              <ChevronDown className="inline h-3 w-3 -mt-0.5" /> show {hiddenCount}{" "}
              more file{hiddenCount === 1 ? "" : "s"}
            </button>
          )}
        </div>
      )}

      {/* Symbols chip row */}
      {summary.symbols.length > 0 && (
        <div className="border-t border-border-soft px-3 py-2.5 space-y-2">
          <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
            Symbols
          </span>
          <ChipGroup>
            {summary.symbols.slice(0, 24).map((s) => (
              <Chip key={s} label={s} asStatic />
            ))}
            {summary.symbols.length > 24 && (
              <Chip
                label={`+${summary.symbols.length - 24} more`}
                asStatic
                className="opacity-70"
              />
            )}
          </ChipGroup>
        </div>
      )}

    </EntitySection>
  );
}

function FileRow({
  bucket,
  onClick,
}: {
  bucket: FileBucket;
  onClick?: () => void;
}) {
  const parts: string[] = [];
  if (bucket.reads) parts.push(`${bucket.reads} read${bucket.reads === 1 ? "" : "s"}`);
  if (bucket.defs) parts.push(`${bucket.defs} def${bucket.defs === 1 ? "" : "s"}`);
  if (bucket.refs)
    parts.push(`${bucket.refs} ref-check${bucket.refs === 1 ? "" : "s"}`);
  if (bucket.extracts)
    parts.push(`${bucket.extracts} extract${bucket.extracts === 1 ? "" : "s"}`);
  const events = parts.length > 0 ? parts.join(" · ") : "—";

  const interactive = !!onClick;
  return (
    <button
      type="button"
      role="listitem"
      disabled={!interactive}
      onClick={onClick}
      className={cn(
        "group flex w-full items-baseline gap-3 px-3 py-2 text-left",
        "border-b border-border-soft last:border-b-0",
        interactive
          ? "cursor-pointer hover:bg-surface-overlay focus-visible:bg-surface-overlay"
          : "cursor-default",
      )}
      title={bucket.file}
    >
      <span className="text-accent shrink-0" aria-hidden="true">◆</span>
      <span className="flex-1 min-w-0 truncate font-mono text-sm text-text">
        {bucket.file}
      </span>
      <span className="shrink-0 whitespace-nowrap font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
        {events}
      </span>
      {interactive && (
        <ChevronRight
          className="h-3.5 w-3.5 shrink-0 text-text-dim group-hover:text-text"
          strokeWidth={2}
        />
      )}
    </button>
  );
}
