import { useEffect, useMemo, useState } from "react";
import {
  listCorpora,
  pickProjectFolder,
  recentActivity,
  registerCorpus,
} from "../../lib/ipc";
import { usePoll } from "../../lib/usePoll";
import { activitySentence } from "../../lib/receipts";
import { Brand } from "../ui/Brand";
import { Beat } from "../ui/Beat";
import { ActionChip } from "../ui/ActionChip";
import { StatusBanner } from "../ui/StatusBanner";
import { IndexingInstrument } from "../ui/IndexingInstrument";
import { useIngestionProgress } from "../../lib/useIngestionProgress";

/**
 * Connect flow — the first five minutes (UX-BLUEPRINT §3.4). Three
 * beats: pick → ministr reads your code → connect your AI. The closing
 * handshake fires ONLY on a genuine first tool call against the new
 * project (a real recent_activity event — never simulated).
 */
type Stage =
  | { kind: "pick" }
  | { kind: "reading"; corpusId: string }
  | { kind: "connect"; corpusId: string };

export function ConnectFlow({ onDone }: { onDone: () => void }) {
  const [stage, setStage] = useState<Stage>({ kind: "pick" });

  return (
    <div className="mx-auto flex min-h-screen max-w-xl flex-col justify-center gap-6 p-8">
      <Brand size="lg" />
      {stage.kind === "pick" ? (
        <PickBeat onPicked={(id) => setStage({ kind: "reading", corpusId: id })} />
      ) : stage.kind === "reading" ? (
        <ReadingBeat
          corpusId={stage.corpusId}
          onIndexed={() => setStage({ kind: "connect", corpusId: stage.corpusId })}
        />
      ) : (
        <ConnectBeat corpusId={stage.corpusId} onDone={onDone} />
      )}
    </div>
  );
}

function PickBeat({ onPicked }: { onPicked: (corpusId: string) => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <section aria-label="pick your project" className="space-y-3">
      <h1 className="text-2xl font-semibold tracking-tight text-ink">
        Point ministr at your project
      </h1>
      <p className="text-sm text-dim">
        Pick the folder you code in. ministr reads it so your AI can see
        it properly — everything stays on your computer.
      </p>
      <ActionChip
        variant="primary"
        disabled={busy}
        onClick={() => {
          setBusy(true);
          void (async () => {
            try {
              const folder = await pickProjectFolder();
              if (!folder) return;
              const res = await registerCorpus([folder]);
              onPicked(res.corpus_id);
            } catch (e) {
              setError(String(e));
            } finally {
              setBusy(false);
            }
          })();
        }}
      >
        Choose a folder…
      </ActionChip>
      {error ? <p className="text-sm text-dim">{error}</p> : null}
    </section>
  );
}

function ReadingBeat({
  corpusId,
  onIndexed,
}: {
  corpusId: string;
  onIndexed: () => void;
}) {
  const { data: corpora } = usePoll(listCorpora, 1_500);
  const mine = corpora?.find((c) => c.id === corpusId);
  // The §7 evolution: sentence + instrument. While determinate progress
  // streams, the full Indexing Instrument carries the moment and the
  // sentence becomes its caption; before that (or if progress is ever
  // unavailable) the original Beat keeps the beat.
  const { progress } = useIngestionProgress(1_000);
  const liveProgress = progress.get(corpusId);

  const sentence = useMemo(() => {
    const st = mine?.status as
      | { state?: string; files_done?: number; files_total?: number }
      | undefined;
    if (st?.state === "indexing" && st.files_total) {
      return `reading your code… ${st.files_done ?? 0} of ${st.files_total} files`;
    }
    return "reading your code…";
  }, [mine]);

  useEffect(() => {
    const st = mine?.status as { state?: string } | undefined;
    if (mine && st?.state === "idle" && mine.files_indexed > 0) onIndexed();
  }, [mine, onIndexed]);

  return (
    <section aria-label="ministr is reading your code" className="space-y-3">
      <h1 className="text-2xl font-semibold tracking-tight text-ink">
        ministr is reading your code
      </h1>
      {liveProgress?.running ? (
        <div className="space-y-2">
          <IndexingInstrument progress={liveProgress} />
          <p className="text-sm text-dim">{sentence}</p>
        </div>
      ) : (
        <Beat sentence={sentence} />
      )}
    </section>
  );
}

const SNIPPET = `claude mcp add ministr -- ministr`;
const SNIPPET_JSON = `{ "mcpServers": { "ministr": { "command": "ministr" } } }`;

function ConnectBeat({
  corpusId,
  onDone,
}: {
  corpusId: string;
  onDone: () => void;
}) {
  // The handshake: ONLY a real tool call against this corpus fires it.
  const { data: activity } = usePoll(() => recentActivity(20), 2_000);
  const first = activity?.find((e) => e.corpus_id === corpusId);

  if (first) {
    return (
      <section aria-label="connected" className="space-y-4">
        <StatusBanner
          state="ok"
          headline="Your AI just saw your code"
          sub={`its first look: ${activitySentence(first).replace("your AI ", "")}`}
        />
        <ActionChip variant="primary" onClick={onDone}>
          Open your project
        </ActionChip>
      </section>
    );
  }

  return (
    <section aria-label="connect your AI" className="space-y-3">
      <h1 className="text-2xl font-semibold tracking-tight text-ink">
        Connect your AI
      </h1>
      <p className="text-sm text-dim">
        In Claude Code, run this once in your project folder:
      </p>
      <pre className="overflow-x-auto rounded-lg border border-line bg-sunken p-3 font-mono text-sm text-ink">
        {SNIPPET}
      </pre>
      <p className="text-sm text-dim">
        Cursor or Windsurf? Add this to your MCP settings instead:
      </p>
      <pre className="overflow-x-auto rounded-lg border border-line bg-sunken p-3 font-mono text-xs text-ink">
        {SNIPPET_JSON}
      </pre>
      <p className="text-sm text-dim">
        This screen will light up the moment your AI takes its first look.
      </p>
    </section>
  );
}
