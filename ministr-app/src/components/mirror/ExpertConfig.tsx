import { useEffect, useState } from "react";
import { listSupportedModels, setCorpusConfig } from "../../lib/ipc";
import type { SupportedModel } from "../../lib/ipc";
import { ActionChip } from "../ui/ActionChip";

/**
 * Per-corpus config behind an expert disclosure (parity-gui-v2-rail-config).
 * Internals vocabulary (models, dimensions) lives ONLY inside the
 * collapsed <details> — the rail label above stays in plain words.
 * Save writes .ministr.toml [corpus] and triggers a reindex daemon-side;
 * onSaved lets the Mirror flip its optimistic Catching-up state.
 */
export function ExpertConfig({
  corpusId,
  model,
  onSaved,
}: {
  corpusId: string;
  /** The corpus's current effective embedding model. */
  model: string;
  onSaved?: () => void;
}) {
  const [models, setModels] = useState<SupportedModel[]>([]);
  const [picked, setPicked] = useState(model);
  const [phase, setPhase] = useState<"idle" | "busy" | "failed">("idle");

  useEffect(() => {
    let alive = true;
    void listSupportedModels()
      .then((m) => alive && setModels(m ?? []))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  const save = () => {
    setPhase("busy");
    void setCorpusConfig(corpusId, picked, null, null)
      .then(() => {
        setPhase("idle");
        onSaved?.();
      })
      .catch(() => setPhase("failed"));
  };

  return (
    <details className="text-xs">
      <summary className="cursor-pointer text-dim hover:text-ink">
        how ministr reads this project · expert
      </summary>
      <div className="mt-2 space-y-2">
        <label className="block text-dim">
          embedding model
          <select
            value={picked}
            onChange={(e) => setPicked(e.target.value)}
            className="mt-1 w-full rounded-md border border-line bg-surface px-2 py-1 text-xs text-ink"
          >
            {!models.some((m) => m.name === picked) ? (
              <option value={picked}>{picked}</option>
            ) : null}
            {models.map((m) => (
              <option key={m.name} value={m.name}>
                {m.name}
                {m.code_optimized ? " · code" : ""}
              </option>
            ))}
          </select>
        </label>
        <div className="flex items-center gap-2">
          <ActionChip
            variant="primary"
            busy={phase === "busy"}
            disabled={picked === model && phase !== "failed"}
            onClick={save}
          >
            {phase === "busy"
              ? "Saving…"
              : phase === "failed"
                ? "Couldn’t save — retry"
                : "Save & re-read"}
          </ActionChip>
        </div>
        <p className="text-dim">
          changing this makes ministr re-read the whole project
        </p>
      </div>
    </details>
  );
}
