import { useState } from "react";
import { triggerReindex } from "../../lib/ipc";
import { ActionChip } from "./ActionChip";

/**
 * CatchUp — the app's primary action, with its loop closed
 * (gui-rw-action-feedback). Never fire-and-forget:
 *
 *   idle → busy ("Starting…", disabled) → accepted (parent shows the
 *   optimistic "Catching up…" until real poll data takes over) or
 *   failed ("Couldn't start — retry", announced politely, click retries).
 */
type Phase = "idle" | "busy" | "failed";

export function CatchUp({
  corpusId,
  onAccepted,
}: {
  corpusId: string;
  /** Fired when the daemon accepted the reindex — the screen flips its
   *  banner optimistically (and must yield to real poll data). */
  onAccepted?: () => void;
}) {
  const [phase, setPhase] = useState<Phase>("idle");

  const run = () => {
    setPhase("busy");
    void triggerReindex(corpusId)
      .then(() => {
        setPhase("idle");
        onAccepted?.();
      })
      .catch(() => setPhase("failed"));
  };

  return (
    <span aria-live="polite">
      <ActionChip
        variant="primary"
        busy={phase === "busy"}
        onClick={(e) => {
          e.stopPropagation();
          run();
        }}
      >
        {phase === "busy"
          ? "Starting…"
          : phase === "failed"
            ? "Couldn’t start — retry"
            : "Catch up"}
      </ActionChip>
      {phase === "failed" ? (
        <span className="ml-2 text-xs text-dim">is ministr running?</span>
      ) : null}
    </span>
  );
}
