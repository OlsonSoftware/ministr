import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { cn } from "../../../lib/utils";
import { BrutalPin } from "../../ui/brutal-icons";
import type { SectionDetailOut } from "./internals";

interface Props {
  /** 1-based citation index — matches source_ids[n-1]. */
  n: number;
  /** Resolved source content_id, or undefined if out of range. */
  sourceId: string | undefined;
  /** The corpus this citation lives in. */
  corpusId: string;
  /** Open in the EntityPanel drawer. */
  onOpen: (n: number) => void;
  /** Pin this answer (the surface uses one Pin per answer, not per source).
   *  When omitted the pin button is hidden. */
  onPinAnswer?: () => void;
  /** Whether the parent answer is pinned — flips the chip border colour. */
  pinned?: boolean;
}

/**
 * Inline citation chip with hover-popover preview + Open action.
 *
 * Visually a tight superscript-style chip rendered inline at the end of a
 * cited claim. Hover or focus reveals a popover with the source's heading
 * path and an excerpt. Click drills into the EntityPanel for breadcrumb
 * navigation.
 *
 * Replaces `components/ask/InlineCitation.tsx`. Differences:
 *   - Source-pinning is gone; pinning lives on the answer card now (see
 *     `PinnedAnswers.tsx`).
 *   - Calls the real `read_section` Tauri command, not the no-longer-
 *     registered `get_section_detail`.
 */
export function AskCitation({
  n,
  sourceId,
  corpusId,
  onOpen,
  onPinAnswer,
  pinned = false,
}: Props) {
  const [open, setOpen] = useState(false);
  const [detail, setDetail] = useState<SectionDetailOut | null>(null);
  const [error, setError] = useState<string | null>(null);
  const hideTimer = useRef<number | null>(null);
  const wrapRef = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    if (!open || detail || !sourceId) return;
    let cancelled = false;
    invoke<SectionDetailOut>("read_section", {
      corpusId,
      sectionId: sourceId,
    })
      .then((d) => {
        if (!cancelled) setDetail(d);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [open, detail, sourceId, corpusId]);

  function show() {
    if (hideTimer.current !== null) {
      window.clearTimeout(hideTimer.current);
      hideTimer.current = null;
    }
    setOpen(true);
  }

  function scheduleHide() {
    hideTimer.current = window.setTimeout(() => setOpen(false), 120);
  }

  return (
    <span
      ref={wrapRef}
      className="relative inline-block"
      onMouseEnter={show}
      onMouseLeave={scheduleHide}
      onFocus={show}
      onBlur={scheduleHide}
    >
      <button
        onClick={() => onOpen(n)}
        title={`Source [${n}]`}
        aria-label={`Open source ${n}`}
        className={cn(
          "inline-flex items-center justify-center align-baseline",
          "px-1 min-w-[1.25rem] h-[1.125rem] -translate-y-[1px]",
          "font-mono text-mono-mini font-bold tabular-nums leading-none",
          "cursor-pointer transition-none rounded-sm border",
          pinned
            ? "border-info bg-surface text-info hover:bg-info hover:text-[var(--color-accent-fg-on)]"
            : "border-accent bg-surface text-accent hover:bg-accent hover:text-[var(--color-accent-fg-on)]",
        )}
      >
        {n}
      </button>

      {open && sourceId && (
        <span
          role="tooltip"
          onMouseEnter={show}
          onMouseLeave={scheduleHide}
          className={cn(
            "absolute left-0 top-full mt-1 z-[1300] w-[360px]",
            "border-2 border-border bg-surface shadow-md",
            "ministr-pin-in",
            "block",
          )}
        >
          <header className="flex items-center justify-between gap-2 border-b-2 border-border bg-surface-overlay px-2.5 py-1.5">
            <span className="font-mono text-mono-mini font-semibold uppercase tracking-[0.05em] text-text-dim">
              Source [{n}]
            </span>
            {detail && (
              <span className="font-mono text-mono-mini text-text truncate">
                {detail.heading_path.join(" / ")}
              </span>
            )}
          </header>
          <div className="px-2.5 py-2 max-h-[200px] overflow-y-auto">
            {error ? (
              <p className="font-mono text-mono-mini text-danger">
                Failed to load: {error}
              </p>
            ) : !detail ? (
              <SkeletonLines />
            ) : (
              <pre className="font-mono text-xs leading-snug text-text-muted whitespace-pre-wrap break-words">
                {truncate(detail.text, 480)}
              </pre>
            )}
          </div>
          <footer className="flex items-center gap-1.5 border-t-2 border-border bg-surface-overlay px-2.5 py-1.5">
            {onPinAnswer && (
              <button
                onClick={() => {
                  onPinAnswer();
                  setOpen(false);
                }}
                disabled={pinned}
                className={cn(
                  "inline-flex items-center gap-1 px-2 py-0.5 cursor-pointer transition-none rounded-sm",
                  "font-mono text-mono-mini font-semibold uppercase tracking-[0.05em]",
                  pinned
                    ? "border border-border-soft bg-surface text-text-dim cursor-not-allowed"
                    : "border border-info bg-surface text-info hover:bg-info hover:text-[var(--color-accent-fg-on)]",
                )}
              >
                <BrutalPin className="h-3 w-3" />
                {pinned ? "Pinned" : "Pin answer"}
              </button>
            )}
            <button
              onClick={() => {
                onOpen(n);
                setOpen(false);
              }}
              className={cn(
                "inline-flex items-center gap-1 px-2 py-0.5 cursor-pointer transition-none rounded-sm",
                "font-mono text-mono-mini font-semibold uppercase tracking-[0.05em]",
                "border border-border-soft bg-surface text-text-muted",
                "hover:text-text hover:border-border",
              )}
            >
              Open ↗
            </button>
          </footer>
        </span>
      )}
    </span>
  );
}

function SkeletonLines() {
  return (
    <div className="space-y-1.5">
      {[80, 70, 90, 60].map((w, i) => (
        <div
          key={i}
          className="h-2 bg-surface-overlay motion-data"
          style={{ width: `${w}%` }}
        />
      ))}
    </div>
  );
}

function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  return s.slice(0, max).trimEnd() + "…";
}
