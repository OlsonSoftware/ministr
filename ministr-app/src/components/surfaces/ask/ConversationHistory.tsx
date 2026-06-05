/**
 * ConversationHistory — the Ask surface's right-rail thread history.
 *
 * Replaces the single-shot "pinned answers" headline: every conversation is
 * saved per corpus and resumable. New-conversation (⌘K) lives here too.
 */
import { MessageSquarePlus, History, X } from "@/components/ui/icons";
import { cn } from "../../../lib/utils";
import { transitionInteractive } from "../../../lib/ui-tokens";
import { threadTitle, type Thread } from "./thread";

interface Props {
  threads: Thread[];
  activeId: string | null;
  onNew: () => void;
  onResume: (t: Thread) => void;
  onDelete: (id: string) => void;
}

export function ConversationHistory({
  threads,
  activeId,
  onNew,
  onResume,
  onDelete,
}: Props) {
  return (
    <div className="flex flex-col gap-2 min-h-0">
      <button
        onClick={onNew}
        className={cn(
          "flex items-center gap-2 rounded-md border border-border bg-surface px-2.5 py-1.5",
          "font-sans text-sm font-medium text-text cursor-pointer",
          "hover:bg-surface-overlay hover:border-border-hover",
          transitionInteractive,
        )}
      >
        <MessageSquarePlus className="h-4 w-4 text-accent" strokeWidth={2.2} />
        New conversation
        <kbd className="ml-auto font-mono text-mono-micro text-text-dim border border-border-soft rounded px-1 py-0.5">
          ⌘K
        </kbd>
      </button>

      <div className="flex items-center gap-2 shrink-0 pt-1">
        <History className="h-3 w-3 text-text-dim" strokeWidth={2} />
        <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
          History
        </span>
        {threads.length > 0 && (
          <span className="font-mono text-mono-mini tabular-nums text-text-dim">
            ({threads.length})
          </span>
        )}
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto flex flex-col gap-1.5">
        {threads.length === 0 ? (
          <p className="font-sans text-xs text-text-dim leading-snug">
            Your conversations show up here — resume any of them with one click.
          </p>
        ) : (
          threads.map((t) => {
            const active = t.id === activeId;
            return (
              <div
                key={t.id}
                className={cn(
                  "group flex items-start gap-1.5 rounded-md border bg-surface p-2",
                  transitionInteractive,
                  active
                    ? "border-accent bg-surface-overlay"
                    : "border-border-soft hover:border-border hover:bg-surface-overlay",
                )}
              >
                <button
                  onClick={() => onResume(t)}
                  className="flex-1 min-w-0 text-left cursor-pointer"
                >
                  <span className="block font-sans text-xs text-text truncate">
                    {threadTitle(t)}
                  </span>
                  <span className="font-mono text-mono-mini text-text-dim tabular-nums">
                    {t.turns.length} turn{t.turns.length === 1 ? "" : "s"}
                  </span>
                </button>
                <button
                  onClick={() => onDelete(t.id)}
                  title="Delete conversation"
                  aria-label={`Delete conversation "${threadTitle(t)}"`}
                  className={cn(
                    "shrink-0 grid h-5 w-5 place-items-center rounded-md cursor-pointer",
                    "text-text-dim hover:text-danger hover:bg-surface",
                    "opacity-0 group-hover:opacity-100",
                    transitionInteractive,
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
