import { ArrowUpRight, FolderPlus, MessageSquare, Sparkles } from "lucide-react";
import { Button } from "../../ui/button";
import { EmptyState } from "../../ui/empty-state";
import { cn } from "../../../lib/utils";
import { headingDisplay, labelMicro } from "../../../lib/ui-tokens";

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

  // ── The "ready" hero — the flagship's front door. A vertically-centered
  //    command-deck canvas (glowing medallion + display headline + value
  //    line + premium starter cards) so the first impression has presence
  //    instead of a top-hugging strip over a dead void. Tone colour stays on
  //    NON-TEXT (the medallion glyph/glow + the hover arrow); the headline,
  //    subtitle and starter text keep full-contrast text-* for AA. ──────────
  return (
    <div className="flex flex-1 min-h-0 flex-col items-center justify-center gap-8 px-4 py-8 text-center">
      <div className="flex flex-col items-center gap-4">
        <span
          aria-hidden
          className={cn(
            "relative grid h-14 w-14 shrink-0 place-items-center rounded-2xl",
            "border border-accent/50 bg-surface-overlay text-accent",
            "shadow-[var(--glow-soft)]",
          )}
        >
          <Sparkles className="h-6 w-6" strokeWidth={2} />
        </span>
        <div className="flex flex-col items-center gap-1.5">
          <h2 className={headingDisplay}>Ask this codebase anything</h2>
          <p className="max-w-md font-sans text-sm text-text-dim leading-relaxed">
            Grounded answers, synthesized from what&rsquo;s actually indexed —
            every claim carries a citation you can open.
          </p>
        </div>
      </div>

      <div className="w-full max-w-2xl">
        <div className="mb-3 flex items-center gap-2.5">
          <span className="h-px flex-1 bg-border-soft" />
          <span className={cn(labelMicro, "inline-flex items-center gap-1.5")}>
            <Sparkles className="h-3 w-3 text-accent" strokeWidth={2.5} aria-hidden />
            Try asking
          </span>
          <span className="h-px flex-1 bg-border-soft" />
        </div>
        <div className="grid grid-cols-1 gap-2.5 @min-[680px]/page:grid-cols-2">
          {STARTERS.map((s) => (
            <button
              key={s}
              onClick={() => props.onApply(s)}
              disabled={props.disabled}
              className={cn(
                "group flex items-center gap-3 rounded-xl text-left",
                "border border-border bg-surface-raised px-4 py-3 shadow-xs",
                "hover:border-border-hover hover:bg-surface-overlay hover:shadow-sm",
                "disabled:opacity-50 disabled:shadow-none",
                "disabled:hover:border-border disabled:hover:bg-surface-raised",
                "cursor-pointer disabled:cursor-not-allowed",
                "transition-all duration-150 ease-out",
              )}
            >
              <span className="font-sans text-sm text-text leading-snug">
                {s}
              </span>
              <ArrowUpRight
                aria-hidden
                strokeWidth={2.5}
                className={cn(
                  "ml-auto h-4 w-4 shrink-0 text-text-muted opacity-0",
                  "transition-all duration-150 ease-out",
                  "group-hover:opacity-100 group-hover:text-accent",
                )}
              />
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
