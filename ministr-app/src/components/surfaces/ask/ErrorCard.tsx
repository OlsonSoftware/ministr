import { AlertTriangle, RefreshCw, TerminalSquare } from "@/components/ui/icons";
import { Button } from "../../ui/button";
import type { InferenceHealth } from "./internals";

/**
 * Error state for an Ask turn — a command-deck FAULT panel. Distinguishes an
 * inference (Claude CLI) failure from a generic ask failure and offers a retry.
 *
 * Mirrors the thread's command-deck language (medallion + paired left-spine)
 * but in a QUIET danger tone: the medallion does not glow — a fault should read
 * as calm and diagnostic, not alarming. A danger left-spine pairs the fault to
 * the question above it, the way the answer card's accent spine does.
 */
export function ErrorCard({
  message,
  onRetry,
  health,
}: {
  message: string;
  onRetry: () => void;
  health: InferenceHealth | null;
}) {
  const isInferenceFailure =
    !health?.available || /inference|claude|spawn|ENOENT/i.test(message);
  const cause = health && !health.available ? health.reason : message;

  return (
    <div className="flex gap-3">
      {/* Danger left-spine — pairs the fault to its question, mirroring the
          answer card's accent spine and SourceDropBlock's info rail. */}
      <span
        aria-hidden
        className="mt-1 w-0.5 self-stretch shrink-0 rounded-full bg-danger/60"
      />
      <div
        role="alert"
        className="min-w-0 flex-1 space-y-3 rounded-lg border border-danger/40 bg-danger/5 p-4"
      >
        <div className="flex items-start gap-3">
          {/* Quiet danger medallion — no glow (error reads calm, not alarming). */}
          <span
            aria-hidden
            className="grid h-11 w-11 shrink-0 place-items-center rounded-xl border border-danger/50 bg-danger/10 text-danger"
          >
            <AlertTriangle className="h-[18px] w-[18px]" strokeWidth={2} />
          </span>
          <div className="min-w-0 flex-1">
            <p className="text-[15px] font-semibold leading-tight text-danger">
              {isInferenceFailure ? "Inference failed" : "Ask failed"}
            </p>
            <p className="mt-1 break-words font-sans text-sm text-text-muted">
              {cause}
            </p>
          </div>
          {/* Retry is the single recovery action — give it the default size
              (legible label, clear affordance), not the cramped sm. */}
          <Button
            variant="outline"
            onClick={onRetry}
            className="shrink-0"
          >
            <RefreshCw className="h-3.5 w-3.5" strokeWidth={2} />
            Retry
          </Button>
        </div>

        {/* Promoted remediation — an inset "fix it" block instead of loose text. */}
        {isInferenceFailure && (
          <div className="flex items-start gap-2.5 rounded-md border border-border-soft bg-surface px-3 py-2.5">
            <TerminalSquare
              className="mt-0.5 h-3.5 w-3.5 shrink-0 text-text-dim"
              strokeWidth={2}
              aria-hidden
            />
            <p className="font-sans text-sm leading-relaxed text-text-dim">
              Ask uses the Claude CLI for synthesis. Install it from{" "}
              <span className="font-mono text-text-muted">claude.com/code</span>{" "}
              and make sure <code className="font-mono text-text">claude</code> is
              on your PATH.
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
