import { useRef, type RefObject } from "react";
import { ArrowRight, History, Loader2 } from "lucide-react";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";
import type { RecentEntry } from "./internals";

interface Props {
  query: string;
  onChange: (next: string) => void;
  onSubmit: () => void;
  loading: boolean;
  /** When true the input is disabled and the placeholder explains why. */
  disabled: boolean;
  /** Reason copy shown in the placeholder while disabled. */
  disabledReason?: string;
  /** Recent answers for this corpus, most-recent first. Empty list hides
   *  the strip entirely. */
  recent: RecentEntry[];
  onPickRecent: (entry: RecentEntry) => void;
  onClearRecent: () => void;
  inputRef?: RefObject<HTMLTextAreaElement | null>;
}

/**
 * Text box + submit button + recent-questions strip.
 *
 * Combines what used to be three separate widgets in `AskView` (Omnibar,
 * RecentStrip, Starters) into one input-row plus an inline horizontal
 * recent strip. Starter questions move to `AskEmpty.tsx` (only shown when
 * the user has never asked anything).
 */
export function AskInput({
  query,
  onChange,
  onSubmit,
  loading,
  disabled,
  disabledReason,
  recent,
  onPickRecent,
  onClearRecent,
  inputRef,
}: Props) {
  const internalRef = useRef<HTMLTextAreaElement>(null);
  const ref = inputRef ?? internalRef;

  function onKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    // ⌘⏎ / Ctrl+⏎ always submits. Plain ⏎ submits unless Shift is held.
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey || !e.shiftKey)) {
      e.preventDefault();
      onSubmit();
    }
  }

  return (
    <div className="flex flex-col gap-2 shrink-0">
      <form
        onSubmit={(e) => {
          e.preventDefault();
          onSubmit();
        }}
      >
        <div className="flex items-start gap-2">
          <textarea
            ref={ref}
            value={query}
            onChange={(e) => onChange(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder={
              disabled
                ? (disabledReason ?? "Ask is unavailable.")
                : "Ask anything about this project. ⏎ to submit, ⇧⏎ for newline."
            }
            rows={2}
            autoFocus
            spellCheck={false}
            disabled={disabled}
            className={cn(
              "min-h-[3.25rem] flex-1 rounded-lg border border-border bg-surface px-3.5 py-2.5",
              "text-base font-sans text-text placeholder:text-text-dim",
              "placeholder:normal-case focus:outline-none focus:border-accent",
              "focus:shadow-[var(--glow-soft)] transition-[border-color,box-shadow] duration-200 resize-none",
              "disabled:opacity-60 disabled:cursor-not-allowed",
            )}
          />
          <Button
            type="submit"
            size="lg"
            disabled={loading || disabled || !query.trim()}
          >
            {loading ? (
              <Loader2 className="h-4 w-4 animate-spin" strokeWidth={2.5} />
            ) : (
              <ArrowRight className="h-4 w-4" strokeWidth={2.5} />
            )}
            {loading ? "Asking" : "Ask"}
          </Button>
        </div>
      </form>

      {recent.length > 0 && (
        <RecentStrip
          recent={recent}
          onPick={onPickRecent}
          onClear={onClearRecent}
          disabled={loading || disabled}
        />
      )}
    </div>
  );
}

function RecentStrip({
  recent,
  onPick,
  onClear,
  disabled,
}: {
  recent: RecentEntry[];
  onPick: (e: RecentEntry) => void;
  onClear: () => void;
  disabled: boolean;
}) {
  return (
    <div className="flex items-center gap-2 overflow-x-auto py-0.5">
      <History
        className="h-3 w-3 text-text-dim shrink-0"
        strokeWidth={2}
        aria-hidden
      />
      <span className="font-mono text-mono-mini uppercase tracking-[0.05em] text-text-dim shrink-0">
        Recent
      </span>
      <div className="flex items-center gap-1.5">
        {recent.slice(0, 6).map((e) => (
          <button
            key={`${e.ts}-${e.query}`}
            onClick={() => onPick(e)}
            disabled={disabled}
            title={e.query}
            className={cn(
              "shrink-0 max-w-[220px] truncate rounded-full",
              "border border-border bg-surface px-2.5 py-0.5",
              "font-sans text-xs text-text-muted hover:text-text hover:border-border-hover hover:bg-surface-overlay",
              "cursor-pointer transition-colors duration-150",
              "disabled:opacity-50 disabled:cursor-not-allowed",
            )}
          >
            {e.query}
          </button>
        ))}
      </div>
      <span className="flex-1" />
      <button
        onClick={onClear}
        disabled={disabled}
        title="Clear recent"
        className={cn(
          "shrink-0 border border-transparent px-1.5 py-0.5",
          "font-mono text-mono-mini uppercase tracking-[0.05em] text-text-dim",
          "hover:text-text hover:border-border-soft",
          "cursor-pointer transition-none rounded-sm",
          "disabled:opacity-50 disabled:cursor-not-allowed",
        )}
      >
        Clear
      </button>
    </div>
  );
}
