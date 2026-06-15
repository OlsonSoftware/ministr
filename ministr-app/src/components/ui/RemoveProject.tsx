import { useState } from "react";
import { removeProject } from "../../lib/ipc";
import { ActionChip } from "./ActionChip";

/**
 * RemoveProject — the missing CRUD verb (gui-ux-remove-project). You could
 * add a project but never forget one. This is a destructive action, so it
 * is confirm-guarded: a quiet "Remove project" opens an explicit
 * consequence + a named "Forget {name}" action sat beside an easy Cancel
 * (Nielsen #3 user control / #5 error prevention; friction-for-safety).
 *
 * Calm identity: no red hue — the plain-words consequence and the explicit
 * verb+noun label carry the weight, not a second colour.
 */
type Phase = "idle" | "confirm" | "busy" | "failed";

export function RemoveProject({
  corpusId,
  displayName,
  onRemoved,
}: {
  corpusId: string;
  displayName: string;
  /** Fired once the daemon has forgotten the project (return Home — it's
   *  gone from the list). */
  onRemoved: () => void;
}) {
  const [phase, setPhase] = useState<Phase>("idle");

  if (phase === "idle") {
    return (
      <ActionChip
        aria-label={`remove ${displayName}`}
        onClick={() => setPhase("confirm")}
      >
        Remove project
      </ActionChip>
    );
  }

  const run = () => {
    setPhase("busy");
    void removeProject(corpusId)
      .then(onRemoved)
      .catch(() => setPhase("failed"));
  };

  return (
    <div
      className="space-y-2"
      role="group"
      aria-label={`remove ${displayName} confirmation`}
    >
      <p className="text-sm text-dim">
        Forget {displayName}? Your AI stops seeing it and the local index is
        deleted. You can add it again any time.
      </p>
      <div className="flex items-center gap-2">
        <ActionChip busy={phase === "busy"} onClick={run}>
          {phase === "failed" ? "Couldn’t remove — retry" : `Forget ${displayName}`}
        </ActionChip>
        <ActionChip disabled={phase === "busy"} onClick={() => setPhase("idle")}>
          Cancel
        </ActionChip>
      </div>
      {phase === "failed" ? (
        <p className="text-xs text-dim">is ministr running?</p>
      ) : null}
    </div>
  );
}
