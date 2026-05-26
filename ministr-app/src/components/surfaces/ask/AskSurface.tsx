/**
 * AskSurface — the codebase Q&A destination, top-level surface.
 *
 * Replaces `components/AskView.tsx`. Same Tauri command (`ask_corpus`)
 * and same Channel-based phase stream — but the visible UI is rebuilt
 * around three principles from the IA reset:
 *
 *   1. Plain-English status. The 5+ pipeline phases (analyzing, retrieving,
 *      reranking, synthesizing, verifying) collapse into three perceptible
 *      states ("Thinking…" / "Writing answer…" / "Checking sources…").
 *      Internal jargon — HyDE, sub_questions, symbol_hints, by_strategy,
 *      bridge_relevant — is dropped from this surface entirely. Anyone
 *      who needs it can open Settings → Developer Tools.
 *
 *   2. Pinned answers, not investigation tabs. Pinning is one-click on
 *      an answer card; the saved set lives on a side panel. The old
 *      multi-tab Investigation system is no longer wired up here — the
 *      hook still exists and remains used by other surfaces, but the
 *      Ask UX is simpler.
 *
 *   3. One detail surface. Citation chips and source rows resolve into
 *      the global EntityPanel; the surface owns no inline detail panes.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { AlertTriangle, RefreshCw, Sparkles } from "lucide-react";

import type { CorpusInfo, DaemonStatus } from "../../../lib/types";
import { Button } from "../../ui/button";
import { AdaptiveSurface } from "../../ui/adaptive-surface";
import { corpusLabel } from "../../../lib/corpus";
import { cn } from "../../../lib/utils";

import { AskAnswer } from "./AskAnswer";
import { AskEmpty } from "./AskEmpty";
import { AskInput } from "./AskInput";
import { AskStatus } from "./AskStatus";
import { PinnedAnswers } from "./PinnedAnswers";
import {
  isLoadingPhase,
  loadPinned,
  loadRecent,
  PINNED_LIMIT,
  RECENT_LIMIT,
  savePinned,
  saveRecent,
  type AskPhase,
  type AskPhaseName,
  type InferenceHealth,
  type RecentEntry,
} from "./internals";

interface Props {
  status: DaemonStatus;
  activeCorpusId: string | null;
}

export function AskSurface({ status, activeCorpusId }: Props) {
  const corpusId = activeCorpusId ?? status.corpora[0]?.id ?? "";
  const corpus = useMemo(
    () => status.corpora.find((c) => c.id === corpusId) ?? null,
    [status.corpora, corpusId],
  );

  const [query, setQuery] = useState("");
  const [phase, setPhase] = useState<AskPhaseName>("idle");
  const [error, setError] = useState<string | null>(null);
  const [verifiedUnsupported, setVerifiedUnsupported] = useState<
    string[] | null
  >(null);
  const [done, setDone] = useState<RecentEntry | null>(null);
  const [recent, setRecent] = useState<RecentEntry[]>([]);
  const [pinned, setPinned] = useState<RecentEntry[]>([]);
  const [health, setHealth] = useState<InferenceHealth | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  // Reset per-corpus state on switch.
  useEffect(() => {
    if (!corpusId) {
      setRecent([]);
      setPinned([]);
    } else {
      setRecent(loadRecent(corpusId));
      setPinned(loadPinned(corpusId));
    }
    setQuery("");
    resetTransient();
  }, [corpusId]);

  // Probe inference health once on mount.
  useEffect(() => {
    let cancelled = false;
    invoke<InferenceHealth>("inference_health").then((h) => {
      if (!cancelled) setHealth(h);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  function resetTransient() {
    setPhase("idle");
    setError(null);
    setVerifiedUnsupported(null);
    setDone(null);
  }

  const submit = useCallback(
    async (raw: string) => {
      const q = raw.trim();
      if (!q || !corpusId) return;
      resetTransient();
      setQuery(q);
      setPhase("analyzing");

      const channel = new Channel<AskPhase>();
      channel.onmessage = (event: AskPhase) => {
        switch (event.kind) {
          case "cache_hit":
            setPhase("synthesizing");
            return;
          case "analyzed":
            setPhase("retrieving");
            return;
          case "retrieved_candidates":
            setPhase("reranking");
            return;
          case "reranked":
          case "retrieved":
            setPhase("synthesizing");
            return;
          case "verified":
            setVerifiedUnsupported(event.unsupported_claims);
            setPhase("verifying");
            return;
          case "done": {
            const entry: RecentEntry = {
              query: q,
              answer: event.answer,
              source_ids: event.source_ids,
              cached: event.cached,
              model: event.model,
              elapsed_ms: event.elapsed_ms,
              ts: Date.now(),
            };
            setDone(entry);
            setPhase("done");
            setRecent((prev) => {
              const next = [
                entry,
                ...prev.filter(
                  (e) => e.query.toLowerCase() !== q.toLowerCase(),
                ),
              ].slice(0, RECENT_LIMIT);
              saveRecent(corpusId, next);
              return next;
            });
            return;
          }
          case "error":
            setError(event.message);
            setPhase("error");
            return;
        }
      };

      try {
        await invoke("ask_corpus", {
          corpusId,
          query: q,
          progress: channel,
        });
      } catch (e) {
        // The Channel error event already sets phase=error in most cases;
        // this catch covers transport/permission failures before any phase
        // event arrives.
        setError((prev) => prev ?? String(e));
        setPhase((prev) => (prev === "error" ? prev : "error"));
      }
    },
    [corpusId],
  );

  function applyStarter(s: string) {
    setQuery(s);
    submit(s);
  }

  function restoreRecent(e: RecentEntry) {
    setQuery(e.query);
    submit(e.query);
  }

  function clearRecent() {
    setRecent([]);
    saveRecent(corpusId, []);
  }

  function pinCurrent() {
    if (!done || !corpusId) return;
    setPinned((prev) => {
      if (prev.some((e) => e.query.toLowerCase() === done.query.toLowerCase())) {
        return prev;
      }
      const next = [done, ...prev].slice(0, PINNED_LIMIT);
      savePinned(corpusId, next);
      return next;
    });
  }

  function unpin(entry: RecentEntry) {
    setPinned((prev) => {
      const next = prev.filter(
        (e) => e.query.toLowerCase() !== entry.query.toLowerCase(),
      );
      savePinned(corpusId, next);
      return next;
    });
  }

  const isPinned = useMemo(() => {
    if (!done) return false;
    return pinned.some(
      (e) => e.query.toLowerCase() === done.query.toLowerCase(),
    );
  }, [pinned, done]);

  // ── No project state — surface owns its own empty handling. ────────────
  if (!corpus) {
    return (
      <AdaptiveSurface>
      <div className="h-full p-5">
      <AskEmpty
        variant="no-project"
        onAddProject={() => {
          window.dispatchEvent(
            new CustomEvent("ministr-navigate", { detail: "projects" }),
          );
        }}
      />
      </div>
      </AdaptiveSurface>
    );
  }

  const inferenceDown = health !== null && !health.available;

  return (
    <AdaptiveSurface>
    <div className="@container/page flex h-full gap-4 min-h-0 p-5">
      <div className="flex-1 min-w-0 flex flex-col gap-3 min-h-0">
        <Header corpus={corpus} />

        <AskInput
          inputRef={inputRef}
          query={query}
          onChange={setQuery}
          onSubmit={() => submit(query)}
          loading={isLoadingPhase(phase)}
          disabled={inferenceDown}
          disabledReason={
            inferenceDown
              ? "Install the Claude CLI to enable Ask…"
              : undefined
          }
          recent={recent}
          onPickRecent={restoreRecent}
          onClearRecent={clearRecent}
        />

        {phase !== "idle" && phase !== "error" && phase !== "done" && (
          <AskStatus phase={phase} />
        )}

        {phase === "done" && done?.cached && <AskStatus phase="done" cached />}

        <div className="flex-1 min-h-0 overflow-y-auto pr-1">
          {phase === "idle" && !done && (
            <>
              {inferenceDown ? (
                <AskEmpty
                  variant="inference-unavailable"
                  reason={health?.reason ?? ""}
                />
              ) : (
                <AskEmpty
                  variant="ready"
                  onApply={applyStarter}
                  disabled={inferenceDown}
                />
              )}
            </>
          )}

          {phase === "error" && error && (
            <ErrorCard
              message={error}
              onRetry={() => submit(query)}
              health={health}
            />
          )}

          {phase === "done" && done && (
            <AskAnswer
              entry={done}
              corpusId={corpusId}
              corpus={corpus}
              verifiedUnsupported={verifiedUnsupported}
              pinned={isPinned}
              onPin={pinCurrent}
              onUnpin={() => unpin(done)}
            />
          )}
        </div>
      </div>

      <aside
        className={cn(
          "hidden @min-[1180px]/page:flex w-[260px] shrink-0",
          "flex-col gap-3 min-h-0 border-l border-border-soft pl-4",
        )}
      >
        <PinnedAnswers
          entries={pinned}
          activeQuery={done?.query ?? query}
          onPick={restoreRecent}
          onUnpin={unpin}
        />
      </aside>
    </div>
    </AdaptiveSurface>
  );
}

function Header({ corpus }: { corpus: CorpusInfo }) {
  return (
    <div className="flex items-center justify-between gap-3 shrink-0">
      <div className="flex items-baseline gap-3 min-w-0">
        <Sparkles
          className="h-4 w-4 text-accent shrink-0 self-center"
          strokeWidth={2.5}
        />
        <h1 className="font-sans text-2xl font-bold text-text leading-none">
          Ask
        </h1>
        <span className="font-mono text-xs uppercase tracking-[0.08em] text-text-dim truncate">
          {corpusLabel(corpus)}
        </span>
      </div>
    </div>
  );
}

function ErrorCard({
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
    <div role="alert" className="rounded-lg border border-danger/40 bg-danger/5 p-4 flex items-start gap-3">
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
