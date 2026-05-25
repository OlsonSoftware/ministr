import { X, Zap } from "lucide-react";
import { cn } from "../../../lib/utils";
import { BrutalPin } from "../../ui/brutal-icons";
import { formatDuration, type RecentEntry } from "./internals";

interface Props {
  entries: RecentEntry[];
  /** Currently displayed answer's query, used to highlight the matching
   *  pinned row. Empty string disables highlight. */
  activeQuery: string;
  onPick: (entry: RecentEntry) => void;
  onUnpin: (entry: RecentEntry) => void;
}

/**
 * Pinned answers — a per-corpus list of answers the user explicitly saved.
 *
 * Replaces the multi-tab `InvestigationTabs` system from the old AskView.
 * Pinning is one-click on the Answer card; this panel surfaces the saved
 * set so the user can jump back to a previous answer (the daemon's answer
 * cache makes the round-trip near-instant).
 *
 * M1 stub: localStorage-backed list, single column, no folders or tags.
 * If users miss multi-tab investigation workflows we can extend later;
 * the plan flagged this regression as worth verifying.
 */
export function PinnedAnswers({
  entries,
  activeQuery,
  onPick,
  onUnpin,
}: Props) {
  return (
    <div className="flex flex-col gap-2 min-h-0">
      <div className="flex items-center gap-2 shrink-0">
        <BrutalPin className="h-3 w-3 text-text-dim" />
        <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
          Pinned
        </span>
        {entries.length > 0 && (
          <span className="font-mono text-mono-mini tabular-nums text-text-dim">
            ({entries.length})
          </span>
        )}
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto flex flex-col gap-1.5">
        {entries.length === 0 ? (
          <p className="font-sans text-xs text-text-dim leading-snug">
            Pin answers you want to keep — they&apos;ll show up here for
            instant recall.
          </p>
        ) : (
          entries.map((e) => {
            const active =
              activeQuery.trim().toLowerCase() === e.query.trim().toLowerCase();
            return (
              <div
                key={`${e.ts}-${e.query}`}
                className={cn(
                  "group flex items-start gap-1.5 border bg-surface p-2",
                  "transition-colors duration-150 ease-out",
                  active
                    ? "border-info bg-surface-overlay"
                    : "border-border-soft hover:border-border hover:bg-surface-overlay",
                )}
              >
                <button
                  onClick={() => onPick(e)}
                  className="flex-1 min-w-0 text-left cursor-pointer"
                >
                  <div className="flex items-center gap-1.5 w-full">
                    {e.cached && (
                      <Zap
                        className="h-3 w-3 text-accent shrink-0"
                        strokeWidth={2.5}
                      />
                    )}
                    <span className="font-sans text-xs text-text truncate flex-1">
                      {e.query}
                    </span>
                  </div>
                  <span className="font-mono text-mono-mini text-text-dim tabular-nums">
                    {e.source_ids.length} src · {formatDuration(e.elapsed_ms)}
                  </span>
                </button>
                <button
                  onClick={() => onUnpin(e)}
                  title="Unpin"
                  aria-label={`Unpin "${e.query}"`}
                  className={cn(
                    "shrink-0 grid h-5 w-5 place-items-center cursor-pointer rounded-md transition-colors duration-150 ease-out",
                    "text-text-dim hover:text-danger hover:bg-surface",
                    "opacity-0 group-hover:opacity-100",
                  )}
                >
                  <X className="h-3 w-3" strokeWidth={2.5} />
                </button>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
