import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AnimatePresence, motion } from "motion/react";
import { popIn } from "../../../lib/motion";
import { glassPanel } from "../../../lib/ui-tokens";
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
  /** Drop this source INTO the thread as a kept block. */
  onDrop?: (n: number) => void;
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
  onDrop,
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
          "cursor-pointer transition-colors duration-150 ease-out rounded-md border",
          pinned
            ? "border-info bg-surface text-info hover:bg-info hover:text-[var(--color-accent-fg-on)]"
            : "border-accent bg-surface text-accent hover:bg-accent hover:text-[var(--color-accent-fg-on)]",
        )}
      >
        {n}
      </button>

      <AnimatePresence>
      {open && sourceId && (
        <motion.span
          role="tooltip"
          onMouseEnter={show}
          onMouseLeave={scheduleHide}
          variants={popIn}
          initial="initial"
          animate="animate"
          exit="exit"
          className={cn(
            // §4 glass tier — the expandable source card is floating chrome,
            // so it reads as translucent layered glass (with the mandatory
            // reduced-transparency solid fallback baked into the token).
            glassPanel,
            "absolute left-0 top-full mt-1 z-[1300] w-[360px] origin-top-left",
            "block overflow-hidden",
          )}
        >
          {/* The popover renders inline inside a markdown <p>, so every
              element here must be phrasing content (a <span>): block tags
              like header/div/footer/pre are invalid descendants of <p> and
              trip React's HTML-nesting validation. Block layout comes from
              display utilities, not block tags. */}
          <span className="flex items-center justify-between gap-2 border-b border-border bg-surface-overlay px-2.5 py-1.5">
            <span className="font-mono text-mono-mini font-semibold uppercase tracking-[0.08em] text-text-dim">
              Source [{n}]
            </span>
            {detail && (
              <span className="font-mono text-mono-mini text-text truncate">
                {detail.heading_path.join(" / ")}
              </span>
            )}
          </span>
          <span className="block px-2.5 py-2 max-h-[200px] overflow-y-auto">
            {error ? (
              <span className="block font-mono text-mono-mini text-danger">
                Failed to load: {error}
              </span>
            ) : !detail ? (
              <SkeletonLines />
            ) : (
              <span className="block font-mono text-xs leading-snug text-text-muted whitespace-pre-wrap break-words">
                {truncate(detail.text, 480)}
              </span>
            )}
          </span>
          <span className="flex items-center gap-1.5 border-t border-border bg-surface-overlay px-2.5 py-1.5">
            {onPinAnswer && (
              <button
                onClick={() => {
                  onPinAnswer();
                  setOpen(false);
                }}
                disabled={pinned}
                className={cn(
                  "inline-flex items-center gap-1 px-2 py-0.5 cursor-pointer transition-colors duration-150 ease-out rounded-md",
                  "font-mono text-mono-mini font-semibold uppercase tracking-[0.08em]",
                  pinned
                    ? "border border-border-soft bg-surface text-text-dim cursor-not-allowed"
                    : "border border-info bg-surface text-info hover:bg-info hover:text-[var(--color-accent-fg-on)]",
                )}
              >
                <BrutalPin className="h-3 w-3" />
                {pinned ? "Pinned" : "Pin answer"}
              </button>
            )}
            {onDrop && (
              <button
                onClick={() => {
                  onDrop(n);
                  setOpen(false);
                }}
                title="Keep this source in the thread"
                className={cn(
                  "inline-flex items-center gap-1 px-2 py-0.5 cursor-pointer transition-colors duration-150 ease-out rounded-md",
                  "font-mono text-mono-mini font-semibold uppercase tracking-[0.08em]",
                  "border border-info bg-surface text-info",
                  "hover:bg-info hover:text-[var(--color-accent-fg-on)]",
                )}
              >
                + Thread
              </button>
            )}
            <button
              onClick={() => {
                onOpen(n);
                setOpen(false);
              }}
              className={cn(
                "inline-flex items-center gap-1 px-2 py-0.5 cursor-pointer transition-colors duration-150 ease-out rounded-md",
                "font-mono text-mono-mini font-semibold uppercase tracking-[0.08em]",
                "border border-border-soft bg-surface text-text-muted",
                "hover:text-text hover:border-border",
              )}
            >
              Open ↗
            </button>
          </span>
        </motion.span>
      )}
      </AnimatePresence>
    </span>
  );
}

function SkeletonLines() {
  return (
    <span className="block space-y-1.5">
      {[80, 70, 90, 60].map((w, i) => (
        <span
          key={i}
          className="block h-2 ministr-skeleton"
          style={{ width: `${w}%` }}
        />
      ))}
    </span>
  );
}

function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  return s.slice(0, max).trimEnd() + "…";
}
