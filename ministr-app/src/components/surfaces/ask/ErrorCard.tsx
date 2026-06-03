import { AlertTriangle, RefreshCw } from "lucide-react";
import { Button } from "../../ui/button";
import type { InferenceHealth } from "./internals";

/**
 * Error state for an Ask turn — distinguishes an inference (Claude CLI)
 * failure from a generic ask failure and offers a retry. Extracted from
 * AskSurface so per-turn rendering (AskTurn) can reuse it.
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

  return (
    <div
      role="alert"
      className="rounded-lg border border-danger/40 bg-danger/5 p-4 flex items-start gap-3"
    >
      <AlertTriangle
        className="h-4 w-4 text-danger shrink-0 mt-0.5"
        strokeWidth={2}
      />
      <div className="flex-1 min-w-0">
        <p className="font-sans text-base font-bold text-danger">
          {isInferenceFailure ? "Inference failed" : "Ask failed"}
        </p>
        <p className="font-sans text-sm text-text-muted mt-1 break-words">
          {health && !health.available ? health.reason : message}
        </p>
        {isInferenceFailure && (
          <p className="font-mono text-xs text-text-dim mt-2">
            Ask uses the Claude CLI for synthesis. Install it from{" "}
            <span className="text-text-muted">claude.com/code</span> and make
            sure <code className="text-text">claude</code> is on your PATH.
          </p>
        )}
      </div>
      <Button variant="outline" size="sm" onClick={onRetry}>
        <RefreshCw className="h-3 w-3" strokeWidth={2} />
        Retry
      </Button>
    </div>
  );
}
