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
import { LiveDot } from "../ui/LiveDot";
import { IndexingInstrument } from "../ui/IndexingInstrument";
import { Screen } from "../ui/Screen";
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

export function ConnectFlow({
  onDone,
  firstRun = false,
}: {
  onDone: () => void;
  /** First launch (no projects registered yet): the pick beat leads with
   *  a plain-words welcome that says what ministr is. The add-a-project
   *  path (firstRun=false) skips it — that user already knows. */
  firstRun?: boolean;
}) {
  const [stage, setStage] = useState<Stage>({ kind: "pick" });

  // ConnectFlow was the calm reference (already justify-center). It now
  // composes the same Screen shell as the other three roots for DRY
  // parity + the consistent trust-footer — same centered rhythm, zero
  // regression (Brand rides inside the centered content, no header slot).
  return (
    <Screen width="xl" align="center" gap="lg">
      <Brand size="lg" />
      {stage.kind === "pick" ? (
        <PickBeat
          firstRun={firstRun}
          onPicked={(id) => setStage({ kind: "reading", corpusId: id })}
        />
      ) : stage.kind === "reading" ? (
        <ReadingBeat
          corpusId={stage.corpusId}
          onIndexed={() => setStage({ kind: "connect", corpusId: stage.corpusId })}
        />
      ) : (
        <ConnectBeat corpusId={stage.corpusId} onDone={onDone} />
      )}
    </Screen>
  );
}

function PickBeat({
  onPicked,
  firstRun = false,
}: {
  onPicked: (corpusId: string) => void;
  firstRun?: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <section aria-label="pick your project" className="space-y-3">
      {firstRun ? (
        <p className="text-sm font-medium text-dim">Welcome to ministr</p>
      ) : null}
      <h1 className="text-2xl font-semibold tracking-tight text-ink">
        {firstRun
          ? "Let your AI read your real code"
          : "Point ministr at your project"}
      </h1>
      <p className="text-sm text-dim">
        {firstRun
          ? "ministr reads a project on your Mac so your AI answers from your actual code — not guesses. Pick the folder you code in to start; everything stays on your computer."
          : "Pick the folder you code in. ministr reads it so your AI can see it properly — everything stays on your computer."}
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
  const { data: activity, error } = usePoll(() => recentActivity(20), 2_000);
  const first = activity?.find((e) => e.corpus_id === corpusId);

  // Active verify (gui-ux-connect-verify-troubleshoot): a real re-query, so
  // the user isn't stuck on a passive wait. "checked" with no event yet
  // surfaces the plain-words verdict + troubleshooting below.
  const [check, setCheck] = useState<"idle" | "checking" | "none">("idle");
  // The daemon is unreachable when polls keep failing with nothing to show.
  const daemonDown = error != null && activity == null;

  const verify = () => {
    setCheck("checking");
    void recentActivity(20)
      .then((evs) => {
        // A hit flips us to the connected branch on the next poll (≤2s);
        // otherwise say so honestly and open the troubleshooting.
        setCheck(evs.some((e) => e.corpus_id === corpusId) ? "idle" : "none");
      })
      .catch(() => setCheck("none"));
  };

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

      <div className="flex items-center justify-between gap-3 pt-1">
        <LiveDot label="watching for your AI’s first look…" />
        <ActionChip busy={check === "checking"} onClick={verify}>
          Check connection
        </ActionChip>
      </div>

      {daemonDown ? (
        <StatusBanner
          state="stale"
          headline="ministr isn’t running on this Mac"
          sub="start ministr (or restart this app) — it reconnects automatically"
        />
      ) : check === "none" ? (
        <StatusBanner
          state="stale"
          headline="Your AI hasn’t connected yet"
          sub="that’s normal if you just added it — try the steps below, then Check again"
        />
      ) : null}

      <details className="rounded-lg border border-line bg-surface px-3 py-2">
        <summary className="cursor-pointer text-sm text-dim">
          Not working?
        </summary>
        <ol className="mt-2 list-decimal space-y-1.5 pl-5 text-sm text-dim">
          <li>Make sure ministr is running (the menu-bar icon, or relaunch this app).</li>
          <li>Run the command above inside your project folder.</li>
          <li>Restart your AI — Claude Code, Cursor, or Windsurf — so it loads ministr.</li>
          <li>Ask it anything about your code; the first question connects it.</li>
        </ol>
      </details>

      <button
        type="button"
        onClick={onDone}
        className="cursor-pointer text-sm text-dim underline-offset-2 transition-colors hover:text-ink hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand"
      >
        Skip for now — open your project
      </button>
    </section>
  );
}
