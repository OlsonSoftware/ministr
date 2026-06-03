/**
 * AskTurn — one exchange in the conversation thread: the user's question
 * followed by the answer (AskAnswer), an in-flight status (AskStatus), or an
 * error (ErrorCard). The question uses a distinct treatment from the answer
 * (accent rule + semibold) so user vs assistant turns read apart at a glance
 * (2026 chat-UI norm). Not to be confused with ui/turn-block.tsx (sessions).
 */
import type { CorpusInfo } from "../../../lib/types";
import { AskAnswer } from "./AskAnswer";
import { AskStatus } from "./AskStatus";
import { ErrorCard } from "./ErrorCard";
import type { InferenceHealth } from "./internals";
import type { AskPhaseName } from "./internals";
import type { Turn } from "./thread";

/** The user's question line — accent left-rule, semibold, distinct from the
 *  Card-framed answer below it. */
export function AskQuestion({ text }: { text: string }) {
  return (
    <div className="flex gap-3">
      <span
        className="mt-1 w-0.5 self-stretch shrink-0 rounded-full bg-accent/60"
        aria-hidden
      />
      <p className="font-sans text-lg font-semibold text-text leading-snug">
        {text}
      </p>
    </div>
  );
}

interface Props {
  turn: Turn;
  corpusId: string;
  corpus: CorpusInfo | null;
  health: InferenceHealth | null;
  pinned: boolean;
  onPin: () => void;
  onUnpin: () => void;
  onRetry: () => void;
}

export function AskTurn({
  turn,
  corpusId,
  corpus,
  health,
  pinned,
  onPin,
  onUnpin,
  onRetry,
}: Props) {
  return (
    <div className="flex flex-col gap-3">
      <AskQuestion text={turn.query} />
      {turn.status === "done" && turn.entry && (
        <AskAnswer
          entry={turn.entry}
          corpusId={corpusId}
          corpus={corpus}
          verifiedUnsupported={turn.unsupported ?? null}
          pinned={pinned}
          onPin={onPin}
          onUnpin={onUnpin}
        />
      )}
      {turn.status === "error" && (
        <ErrorCard
          message={turn.error ?? "Ask failed"}
          onRetry={onRetry}
          health={health}
        />
      )}
    </div>
  );
}

/** The in-flight turn: the question plus the live pipeline status. */
export function AskPendingTurn({
  query,
  phase,
}: {
  query: string;
  phase: AskPhaseName;
}) {
  return (
    <div className="flex flex-col gap-3">
      <AskQuestion text={query} />
      <AskStatus phase={phase} />
    </div>
  );
}
