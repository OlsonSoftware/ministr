/**
 * AskSurface — a conversation with your codebase.
 *
 * Each question + answer is a Turn in a scrollable thread (AskTurn); the
 * composer is docked at the bottom; follow-ups carry prior turns as context
 * (buildContextualQuery — a stateless re-ask; true cross-turn retrieval is
 * the daemon follow-on aaa-ask-multiturn-context). The right rail is
 * resumable conversation History (ConversationHistory) over the per-corpus
 * thread store, with the Pinned answers below it. ⌘K starts a fresh thread.
 *
 * Same Tauri command (`ask_corpus`) + Channel phase stream as before; the
 * phase machine now drives the LAST (in-flight) turn rather than a single
 * replace-on-submit answer.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { MessageSquare } from "lucide-react";

import type { DaemonStatus } from "../../../lib/types";
import { AdaptiveSurface } from "../../ui/adaptive-surface";
import { FacetHeader } from "../../ui/facet-header";
import { corpusLabel } from "../../../lib/corpus";
import { cn } from "../../../lib/utils";
import { useWorkspaceOptional } from "../../workspace/WorkspaceContext";

import { AskEmpty } from "./AskEmpty";
import { AskInput } from "./AskInput";
import { AskTurn, AskPendingTurn } from "./AskTurn";
import { ConversationHistory } from "./ConversationHistory";
import { PinnedAnswers } from "./PinnedAnswers";
import {
  buildContextualQuery,
  loadThreads,
  newId,
  saveThreads,
  sourceTurn,
  THREADS_LIMIT,
  type Thread,
  type Turn,
} from "./thread";
import {
  loadPinned,
  savePinned,
  PINNED_LIMIT,
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
  // Cross-facet "Ask about this" intent (Explore → Ask). Optional: AskSurface
  // is also storied in isolation, outside the workspace provider.
  const workspace = useWorkspaceOptional();

  const [query, setQuery] = useState("");
  const [phase, setPhase] = useState<AskPhaseName>("idle");
  const [turns, setTurns] = useState<Turn[]>([]);
  const [pendingQuery, setPendingQuery] = useState<string | null>(null);
  const [pinned, setPinned] = useState<RecentEntry[]>([]);
  const [threads, setThreads] = useState<Thread[]>([]);
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);
  const [health, setHealth] = useState<InferenceHealth | null>(null);

  const inputRef = useRef<HTMLTextAreaElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const turnsRef = useRef<Turn[]>([]);
  const activeIdRef = useRef<string | null>(null);
  const busyRef = useRef(false);

  useEffect(() => {
    turnsRef.current = turns;
  }, [turns]);

  // ── Per-corpus reset + load. ───────────────────────────────────────────
  useEffect(() => {
    if (!corpusId) {
      setThreads([]);
      setPinned([]);
    } else {
      setThreads(loadThreads(corpusId));
      setPinned(loadPinned(corpusId));
    }
    setTurns([]);
    setPendingQuery(null);
    setPhase("idle");
    setActiveThreadId(null);
    activeIdRef.current = null;
    setQuery("");
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

  // ── Persist the current thread whenever a turn lands. ──────────────────
  useEffect(() => {
    if (!corpusId || turns.length === 0) return;
    let id = activeIdRef.current;
    if (!id) {
      id = newId();
      activeIdRef.current = id;
      setActiveThreadId(id);
    }
    const fid = id;
    setThreads((prev) => {
      const existing = prev.find((t) => t.id === fid);
      const thread: Thread = {
        id: fid,
        corpusId,
        turns,
        createdAt: existing?.createdAt ?? Date.now(),
        updatedAt: Date.now(),
      };
      const next = [thread, ...prev.filter((t) => t.id !== fid)].slice(
        0,
        THREADS_LIMIT,
      );
      saveThreads(corpusId, next);
      return next;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [turns]);

  // ── Auto-scroll the thread to the newest turn / status. ────────────────
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [turns.length, pendingQuery, phase]);

  const submit = useCallback(
    async (raw: string) => {
      const q = raw.trim();
      if (!q || !corpusId || busyRef.current) return;
      busyRef.current = true;

      const contextual = buildContextualQuery(turnsRef.current, q);
      setQuery("");
      setPendingQuery(q);
      setPhase("analyzing");

      let settled = false;
      let unsupported: string[] | null = null;

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
            unsupported = event.unsupported_claims;
            setPhase("verifying");
            return;
          case "done": {
            settled = true;
            const entry: RecentEntry = {
              query: q,
              answer: event.answer,
              source_ids: event.source_ids,
              cached: event.cached,
              model: event.model,
              elapsed_ms: event.elapsed_ms,
              ts: Date.now(),
            };
            setTurns((prev) => [
              ...prev,
              { id: newId(), query: q, status: "done", entry, unsupported },
            ]);
            setPendingQuery(null);
            setPhase("done");
            return;
          }
          case "error":
            settled = true;
            setTurns((prev) => [
              ...prev,
              { id: newId(), query: q, status: "error", error: event.message },
            ]);
            setPendingQuery(null);
            setPhase("error");
            return;
        }
      };

      try {
        await invoke("ask_corpus", {
          corpusId,
          query: contextual,
          progress: channel,
        });
      } catch (e) {
        if (!settled) {
          setTurns((prev) => [
            ...prev,
            { id: newId(), query: q, status: "error", error: String(e) },
          ]);
          setPendingQuery(null);
          setPhase("error");
        }
      } finally {
        busyRef.current = false;
      }
    },
    [corpusId],
  );

  function applyStarter(s: string) {
    submit(s);
  }

  // ── Citation drop-in — open a citation INTO the thread as a kept source
  //    block (aaa-ask-citation-dropin). Persists with the thread (the turns
  //    effect), so it survives resume. Deduped: re-opening an already-kept
  //    source is a no-op rather than stacking duplicates. ──────────────────
  const dropSource = useCallback((contentId: string, n?: number) => {
    setTurns((prev) =>
      prev.some((t) => t.kind === "source" && t.source?.contentId === contentId)
        ? prev
        : [...prev, sourceTurn(contentId, n)],
    );
  }, []);

  // ── Cross-facet "Ask about this" — consume the workspace ask-intent. Keyed
  //    on the intent nonce so re-asking the SAME source fires again; runs after
  //    the per-corpus reset above so the dropped source survives a facet swap
  //    that remounts this surface (aaa-explore-integrated). ──────────────────
  const askIntent = workspace?.askIntent;
  const clearAskIntent = workspace?.clearAskIntent;
  useEffect(() => {
    if (!askIntent || !corpusId) return;
    dropSource(askIntent.contentId, askIntent.n);
    clearAskIntent?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [askIntent?.nonce, corpusId]);

  const removeTurn = useCallback((id: string) => {
    setTurns((prev) => prev.filter((t) => t.id !== id));
  }, []);

  // ── Thread lifecycle. ──────────────────────────────────────────────────
  const newThread = useCallback(() => {
    setTurns([]);
    setPendingQuery(null);
    setPhase("idle");
    setActiveThreadId(null);
    activeIdRef.current = null;
    setQuery("");
    inputRef.current?.focus();
  }, []);

  function resumeThread(t: Thread) {
    setTurns(t.turns);
    setActiveThreadId(t.id);
    activeIdRef.current = t.id;
    setPendingQuery(null);
    setPhase("idle");
    setQuery("");
  }

  function deleteThread(id: string) {
    setThreads((prev) => {
      const next = prev.filter((t) => t.id !== id);
      if (corpusId) saveThreads(corpusId, next);
      return next;
    });
    if (id === activeIdRef.current) newThread();
  }

  // ── Pinning (per answer). ──────────────────────────────────────────────
  function pinEntry(entry: RecentEntry) {
    if (!corpusId) return;
    setPinned((prev) => {
      if (prev.some((e) => e.query.toLowerCase() === entry.query.toLowerCase()))
        return prev;
      const next = [entry, ...prev].slice(0, PINNED_LIMIT);
      savePinned(corpusId, next);
      return next;
    });
  }

  function unpin(entry: RecentEntry) {
    setPinned((prev) => {
      const next = prev.filter(
        (e) => e.query.toLowerCase() !== entry.query.toLowerCase(),
      );
      if (corpusId) savePinned(corpusId, next);
      return next;
    });
  }

  const isPinned = (entry: RecentEntry) =>
    pinned.some((e) => e.query.toLowerCase() === entry.query.toLowerCase());

  // ── ⌘K / Ctrl+K starts a fresh thread. ────────────────────────────────
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && (e.key === "k" || e.key === "K")) {
        e.preventDefault();
        newThread();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [newThread]);

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
  const empty = turns.length === 0 && !pendingQuery;

  return (
    <AdaptiveSurface>
      <div className="@container/page flex h-full gap-4 min-h-0 p-5">
        <div className="flex-1 min-w-0 flex flex-col gap-3 min-h-0">
          <FacetHeader
            bare
            icon={MessageSquare}
            title="Ask"
            scope={corpusLabel(corpus)}
          />

          <div className="flex-1 min-h-0 overflow-y-auto pr-1 flex flex-col gap-6">
            {empty &&
              (inferenceDown ? (
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
              ))}

            {turns.map((t) => (
              <AskTurn
                key={t.id}
                turn={t}
                corpusId={corpusId}
                corpus={corpus}
                health={health}
                pinned={t.entry ? isPinned(t.entry) : false}
                onPin={() => t.entry && pinEntry(t.entry)}
                onUnpin={() => t.entry && unpin(t.entry)}
                onRetry={() => submit(t.query)}
                onDropSource={dropSource}
                onRemoveSource={() => removeTurn(t.id)}
              />
            ))}

            {pendingQuery && <AskPendingTurn query={pendingQuery} phase={phase} />}

            <div ref={bottomRef} />
          </div>

          <AskInput
            inputRef={inputRef}
            query={query}
            onChange={setQuery}
            onSubmit={() => submit(query)}
            loading={pendingQuery != null}
            disabled={inferenceDown}
            disabledReason={
              inferenceDown ? "Install the Claude CLI to enable Ask…" : undefined
            }
            recent={[]}
            onPickRecent={() => {}}
            onClearRecent={() => {}}
          />
        </div>

        <aside
          className={cn(
            "hidden @min-[1180px]/page:flex w-[260px] shrink-0",
            "flex-col gap-4 min-h-0 border-l border-border-soft pl-4",
          )}
        >
          <div className="flex-1 min-h-0">
            <ConversationHistory
              threads={threads}
              activeId={activeThreadId}
              onNew={newThread}
              onResume={resumeThread}
              onDelete={deleteThread}
            />
          </div>
          <div className="flex-1 min-h-0">
            <PinnedAnswers
              entries={pinned}
              activeQuery={pendingQuery ?? ""}
              onPick={(e) => submit(e.query)}
              onUnpin={unpin}
            />
          </div>
        </aside>
      </div>
    </AdaptiveSurface>
  );
}
