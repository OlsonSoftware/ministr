import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "../ui/button";
import type { CorpusInfo } from "../../lib/types";

/** One supported embedding model, mirrored from the `list_supported_models`
 *  Tauri command (sourced from core `supported_models()` — no drift). */
interface SupportedModel {
  name: string;
  dimension: number;
  description: string;
  code_optimized: boolean;
}

/** Parse a non-negative integer field; blank → null (leave the knob untouched,
 *  matching `RepoConfig::set_corpus_config`'s `None` semantics). Invalid → null
 *  so a typo can't write garbage. */
function parseKnob(raw: string): number | null {
  const t = raw.trim();
  if (t === "") return null;
  const n = Number(t);
  return Number.isInteger(n) && n >= 0 ? n : null;
}

/**
 * Per-corpus config editor (parity-gui-corpus-config-ui).
 *
 * Lets a GUI-only user pick the corpus's embedding `model` and set the
 * Matryoshka `dimension` / `rerank_depth`, persisting via the `set_corpus_config`
 * command — which writes the SAME `.ministr.toml` `[corpus]` table the CLI and
 * the daemon's config seam read, then re-indexes so the change is honored.
 */
export function CorpusConfigEditor({ corpus }: { corpus: CorpusInfo }) {
  const [models, setModels] = useState<SupportedModel[] | null>(null);
  const [model, setModel] = useState(corpus.model ?? "");
  const [dimension, setDimension] = useState("");
  const [rerankDepth, setRerankDepth] = useState("");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<SupportedModel[]>("list_supported_models")
      .then((m) => {
        if (!cancelled) setModels(m);
      })
      .catch((e) => {
        console.error("[ministr] list_supported_models error:", e);
        if (!cancelled) setModels([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Re-sync the model select if the corpus's effective model changes under us
  // (e.g. after a reindex completes and the status poll refreshes the entity).
  useEffect(() => {
    setModel(corpus.model ?? "");
  }, [corpus.model]);

  const dirty =
    model.trim() !== (corpus.model ?? "").trim() ||
    dimension.trim() !== "" ||
    rerankDepth.trim() !== "";

  async function save() {
    setSaving(true);
    setSaved(false);
    setError(null);
    try {
      await invoke("set_corpus_config", {
        corpusId: corpus.id,
        model: model.trim() === "" ? null : model.trim(),
        dimension: parseKnob(dimension),
        rerankDepth: parseKnob(rerankDepth),
      });
      setSaved(true);
      setDimension("");
      setRerankDepth("");
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  const knownModel = models?.some((m) => m.name === model) ?? false;

  return (
    <div className="space-y-2 border-t border-border-soft pt-2.5 mt-1.5">
      <p className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
        Embedding config
      </p>

      <label className="flex flex-col gap-1">
        <span className="font-mono text-xs text-text-dim">model</span>
        <select
          value={model}
          onChange={(e) => setModel(e.target.value)}
          disabled={saving}
          className="h-8 px-2 rounded-md border border-border bg-surface font-mono text-xs text-text focus:outline-none focus:border-border-hover"
        >
          <option value="">(daemon default)</option>
          {/* Keep an unknown current value selectable so we never silently
              drop a model the daemon set but this list doesn't enumerate. */}
          {model && !knownModel ? <option value={model}>{model}</option> : null}
          {(models ?? []).map((m) => (
            <option key={m.name} value={m.name}>
              {m.name}
              {m.code_optimized ? " · code" : ""} ({m.dimension}d)
            </option>
          ))}
        </select>
      </label>

      <div className="grid grid-cols-2 gap-2">
        <label className="flex flex-col gap-1">
          <span className="font-mono text-xs text-text-dim">dimension</span>
          <input
            inputMode="numeric"
            value={dimension}
            onChange={(e) => setDimension(e.target.value)}
            disabled={saving}
            placeholder="full"
            className="h-8 px-2 rounded-md border border-border bg-surface font-mono text-xs text-text focus:outline-none focus:border-border-hover"
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className="font-mono text-xs text-text-dim">rerank_depth</span>
          <input
            inputMode="numeric"
            value={rerankDepth}
            onChange={(e) => setRerankDepth(e.target.value)}
            disabled={saving}
            placeholder="100"
            className="h-8 px-2 rounded-md border border-border bg-surface font-mono text-xs text-text focus:outline-none focus:border-border-hover"
          />
        </label>
      </div>

      <div className="flex items-center gap-2 pt-0.5">
        <Button size="sm" disabled={saving || !dirty} onClick={save}>
          {saving ? "Reindexing…" : "Save + reindex"}
        </Button>
        {saved ? (
          <span className="font-mono text-xs text-text-dim">
            saved — reindexing
          </span>
        ) : null}
        {error ? (
          <span className="font-mono text-xs text-danger break-all">{error}</span>
        ) : null}
      </div>

      <p className="font-mono text-[10px] leading-snug text-text-dim">
        Writes <code>.ministr.toml</code> [corpus] and re-indexes.
        dimension/rerank_depth only apply to Matryoshka-capable models.
      </p>
    </div>
  );
}
