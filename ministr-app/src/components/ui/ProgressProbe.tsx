import { useIngestionProgress } from "../../lib/useIngestionProgress";

/**
 * Diagnostic readout of [`useIngestionProgress`] (gui-progress-data-hook):
 * renders every derived field as labelled rows with stable test ids so
 * Storybook stories — and the instrument chunks that build on the hook —
 * can assert the math mechanically. Not mounted in the app shell; the
 * user-facing rendering is the Indexing Instrument (gui-indexing-instrument).
 */
export function ProgressProbe({ intervalMs = 300 }: { intervalMs?: number }) {
  const { progress, error } = useIngestionProgress(intervalMs);

  if (error) {
    return <p data-testid="progress-error">progress unavailable: {error}</p>;
  }
  if (progress.size === 0) {
    return <p data-testid="progress-empty">no corpora reporting</p>;
  }
  return (
    <div className="space-y-4 font-mono text-sm">
      {[...progress.values()].map((p) => (
        <dl
          key={p.corpusId}
          data-testid={`progress-${p.corpusId}`}
          className="space-y-1"
        >
          <Row k="corpus" v={p.corpusId} />
          <Row k="phase" v={p.phase} />
          <Row
            k="state"
            v={p.complete ? "complete" : p.running ? "running" : "pending"}
          />
          <Row k="files" v={`${p.filesDone}/${p.filesTotal}`} />
          <Row k="embeddings" v={`${p.embeddingsDone}/${p.embeddingsTotal}`} />
          <Row k="current" v={p.currentFile ?? "—"} />
          <Row
            k="percent"
            v={p.percent === null ? "—" : `${Math.round(p.percent * 100)}%`}
          />
          <Row
            k="rate"
            v={p.ratePerSec === null ? "—" : `${p.ratePerSec.toFixed(0)}/s`}
          />
          <Row
            k="eta"
            v={
              p.stalled
                ? "stalled"
                : p.etaSeconds === null
                  ? "—"
                  : `~${p.etaSeconds}s`
            }
          />
        </dl>
      ))}
    </div>
  );
}

function Row({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex gap-2">
      <dt data-testid={`probe-${k}`} className="w-28 text-dim">
        {k}
      </dt>
      <dd data-testid={`probe-${k}-value`}>{v}</dd>
    </div>
  );
}
