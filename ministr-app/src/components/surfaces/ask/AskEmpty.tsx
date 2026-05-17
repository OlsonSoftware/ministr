import { FolderPlus, MessageSquare, Sparkles } from "lucide-react";
import { Button } from "../../ui/button";
import { EmptyState } from "../../ui/empty-state";
import { cn } from "../../../lib/utils";

const STARTERS = [
  "Give me a tour of the project's architecture.",
  "What are the main entry points?",
  "How does authentication work?",
  "Where are background jobs scheduled?",
  "What database schema does this use?",
  "Which modules are the riskiest to change?",
];

interface ReadyProps {
  variant: "ready";
  onApply: (query: string) => void;
  /** When true the starter buttons are dimmed and click is a no-op. */
  disabled: boolean;
}

interface NoProjectProps {
  variant: "no-project";
  onAddProject: () => void;
}

interface InferenceUnavailableProps {
  variant: "inference-unavailable";
  reason: string;
}

type Props = ReadyProps | NoProjectProps | InferenceUnavailableProps;

/**
 * Empty / pre-question states for the Ask surface.
 *
 *   - "ready"                — a project is selected, inference works,
 *                              user has never asked. Shows starter chips.
 *   - "no-project"           — daemon up, but no project indexed.
 *                              Routes the user toward the Add flow.
 *   - "inference-unavailable" — Claude CLI not installed / on PATH.
 *                              Tells the user what's missing.
 */
export function AskEmpty(props: Props) {
  if (props.variant === "no-project") {
    return (
      <EmptyState
        icon={FolderPlus}
        title="No project to ask about"
        hint="Add a folder so ministr can index it. Once indexing finishes you can ask questions of the codebase."
        action={
          <Button onClick={props.onAddProject}>
            <FolderPlus className="h-4 w-4" strokeWidth={2.5} />
            Add a project
          </Button>
        }
      />
    );
  }

  if (props.variant === "inference-unavailable") {
    return (
      <EmptyState
        icon={MessageSquare}
        title="Ask needs a Claude CLI"
        hint={
          props.reason ||
          "Install the Claude CLI from claude.com/code and make sure 'claude' is on your PATH. Ask uses it for synthesis."
        }
      />
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <Sparkles
          className="h-3.5 w-3.5 text-accent"
          strokeWidth={2.5}
          aria-hidden
        />
        <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
          Try asking
        </span>
        <span className="flex-1 h-px bg-border-soft" />
      </div>
      <div className="grid grid-cols-1 @min-[680px]/page:grid-cols-2 gap-2">
        {STARTERS.map((s) => (
          <button
            key={s}
            onClick={() => props.onApply(s)}
            disabled={props.disabled}
            className={cn(
              "group flex items-start gap-2.5 border border-border-soft bg-surface",
              "px-3 py-2.5 text-left",
              "hover:border-accent hover:bg-surface-overlay",
              "disabled:opacity-50 disabled:hover:border-border-soft disabled:hover:bg-surface",
              "cursor-pointer disabled:cursor-not-allowed transition-colors duration-150 ease-out",
            )}
          >
            <span className="font-mono text-xs font-bold text-accent shrink-0 mt-0.5">
              ?
            </span>
            <span className="font-sans text-sm text-text leading-snug">
              {s}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}
