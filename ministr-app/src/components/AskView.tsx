/**
 * AskView — the codebase Q&A surface.
 *
 * One-shot synthesis (not a chat): the daemon's `ask` pipeline doesn't
 * carry conversation state, and the cache key is `blake3(query)`. So
 * instead of pretending to be conversational, we keep a per-corpus
 * "recent answers" strip — clicking restores instantly via cache.
 *
 * The Tauri `ask_corpus` command streams `AskPhase` events on a Channel
 * so the UI can render retrieving → synthesizing → done with real
 * skeletons that resolve into the actual sources before the answer
 * arrives. Citations are numeric `[N]` markers parsed in-place and
 * routed through the global EntityPanel so the source is one click away.
 */

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle,
  ArrowRight,
  Check,
  Copy,
  ExternalLink,
  History,
  Loader2,
  RefreshCw,
  Sparkles,
  Terminal,
  X,
  Zap,
} from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import type {
  CorpusInfo,
  DaemonStatus,
  SearchResult,
  SymbolDefinitionDetail,
} from "../lib/types";
import { Button } from "./ui/button";
import { Card } from "./ui/card";
import { EmptyState } from "./ui/empty-state";
import { useEntityPanel } from "../hooks/useEntityPanel";
import { corpusLabel } from "../lib/corpus";
import { basename, corpusRelative } from "../lib/path";
import { cn } from "../lib/utils";

// ─────────────────────────────────────────────────────────────────────────────
// Types matching ministr-app/src-tauri/src/commands.rs::AskPhase

type AskPhase =
  | { kind: "cache_hit"; source_ids: string[] }
  | {
      kind: "analyzed";
      sub_questions: string[];
      hyde_preview: string;
      symbol_hints: string[];
      bridge_relevant: boolean;
    }
  | {
      kind: "retrieved_candidates";
      by_strategy: Record<string, number>;
      merged_ids: string[];
    }
  | { kind: "reranked"; source_ids: string[] }
  | { kind: "retrieved"; source_ids: string[] }
  | { kind: "verified"; unsupported_claims: string[] }
  | {
      kind: "done";
      answer: string;
      source_ids: string[];
      cached: boolean;
      model: string;
      elapsed_ms: number;
    }
  | { kind: "error"; message: string };

interface InferenceHealth {
  available: boolean;
  reason: string;
  binary_path: string | null;
}

interface SectionDetailOut {
  section_id: string;
  heading_path: string[];
  text: string;
  summary: string | null;
  claims_available: number;
}

interface RecentEntry {
  query: string;
  answer: string;
  source_ids: string[];
  cached: boolean;
  model: string;
  elapsed_ms: number;
  ts: number;
}

// ─────────────────────────────────────────────────────────────────────────────
// Persistence — recent answers, keyed per-corpus.

const RECENT_STORAGE_KEY = "ministr-ask-recent-v1";
const RECENT_LIMIT = 10;

function loadRecent(corpusId: string): RecentEntry[] {
  try {
    const raw = localStorage.getItem(RECENT_STORAGE_KEY);
    if (!raw) return [];
    const all = JSON.parse(raw) as Record<string, RecentEntry[]>;
    const list = all[corpusId];
    return Array.isArray(list) ? list.slice(0, RECENT_LIMIT) : [];
  } catch {
    return [];
  }
}

function saveRecent(corpusId: string, entries: RecentEntry[]) {
  try {
    const raw = localStorage.getItem(RECENT_STORAGE_KEY);
    const all = (raw ? JSON.parse(raw) : {}) as Record<string, RecentEntry[]>;
    all[corpusId] = entries.slice(0, RECENT_LIMIT);
    localStorage.setItem(RECENT_STORAGE_KEY, JSON.stringify(all));
  } catch {
    /* localStorage unavailable — non-fatal */
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Suggested starter questions — generic but tuned for code Q&A.

const STARTERS = [
  "Give me a tour of the project's architecture.",
  "What are the main entry points?",
  "How does authentication work?",
  "Where are background jobs scheduled?",
  "What database schema does this use?",
  "Which modules are the riskiest to change?",
];

// ─────────────────────────────────────────────────────────────────────────────
// Citation helpers

/** Extract numeric citation references from a markdown answer. Returns the
 *  set of source indices (1-based) that appear in `[N]` or `[N, M]` form. */
function citedIndices(answer: string): Set<number> {
  const set = new Set<number>();
  const re = /\[(\d+(?:\s*,\s*\d+)*)\]/g;
  let match;
  while ((match = re.exec(answer)) !== null) {
    for (const piece of match[1].split(",")) {
      const n = parseInt(piece.trim(), 10);
      if (Number.isFinite(n) && n > 0) set.add(n);
    }
  }
  return set;
}

/** Best-effort: extract a file path from a content_id like
 *  `d:/code/foo/bar.rs#root:c0` or `sym-d:/code/foo/bar.rs::mod::Sym`. */
function filePathFromContentId(id: string): string {
  const noPrefix = id.replace(/^sym-/, "");
  const hashIdx = noPrefix.indexOf("#");
  const colonIdx = noPrefix.indexOf("::");
  let candidate: string;
  if (hashIdx >= 0) candidate = noPrefix.slice(0, hashIdx);
  else if (colonIdx >= 0) candidate = noPrefix.slice(0, colonIdx);
  else candidate = noPrefix;
  return candidate;
}

/** Short label for a source: heading path (with the file-basename
 *  segment trimmed when it duplicates the file tag) > basename of file. */
function sourceLabel(id: string, headingPath?: string[]): string {
  const file = filePathFromContentId(id);
  const fileBase = basename(file);
  const fileStem = fileBase.replace(/\.[^.]+$/, "");
  if (headingPath && headingPath.length > 0) {
    // Heading paths from the indexer often start with the file's
    // basename or stem (e.g. ["auth.rs", "AuthMiddleware"]). When we
    // already render the file-relative tag next to this label, that
    // first segment is dead weight.
    const trimmed =
      headingPath[0] === fileBase || headingPath[0] === fileStem
        ? headingPath.slice(1)
        : headingPath;
    if (trimmed.length > 0) return trimmed.join(" › ");
  }
  return fileBase || id;
}

// ─────────────────────────────────────────────────────────────────────────────
// Top-level component

interface Props {
  status: DaemonStatus;
  activeCorpusId: string | null;
  setActiveCorpusId: (id: string | null) => void;
}

export function AskView({ status, activeCorpusId }: Props) {
  const corpusId = activeCorpusId ?? status.corpora[0]?.id ?? "";
  const corpus = useMemo(
    () => status.corpora.find((c) => c.id === corpusId) ?? null,
    [status.corpora, corpusId],
  );

  const [query, setQuery] = useState("");
  const [phase, setPhase] = useState<
    | "idle"
    | "analyzing"
    | "retrieving"
    | "reranking"
    | "synthesizing"
    | "verifying"
    | "done"
    | "error"
  >("idle");
  const [error, setError] = useState<string | null>(null);
  const [partialSourceIds, setPartialSourceIds] = useState<string[]>([]);
  const [analysis, setAnalysis] = useState<{
    sub_questions: string[];
    hyde_preview: string;
    symbol_hints: string[];
    bridge_relevant: boolean;
  } | null>(null);
  const [byStrategy, setByStrategy] = useState<Record<string, number>>({});
  const [verified, setVerified] = useState<{ unsupported: string[] } | null>(
    null,
  );
  const [done, setDone] = useState<RecentEntry | null>(null);
  const [recent, setRecent] = useState<RecentEntry[]>([]);
  const [health, setHealth] = useState<InferenceHealth | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  // Reset per-corpus state on switch.
  useEffect(() => {
    setRecent(corpusId ? loadRecent(corpusId) : []);
    resetTransient();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [corpusId]);

  // Probe inference health once on mount + whenever we navigate back to
  // this view (cheap PATH check; users install Claude mid-session).
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
    setPartialSourceIds([]);
    setAnalysis(null);
    setByStrategy({});
    setVerified(null);
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
            // Cache hit: skip everything, the answer is right behind this.
            setPartialSourceIds(event.source_ids);
            setPhase("synthesizing");
            return;
          case "analyzed":
            setAnalysis({
              sub_questions: event.sub_questions,
              hyde_preview: event.hyde_preview,
              symbol_hints: event.symbol_hints,
              bridge_relevant: event.bridge_relevant,
            });
            setPhase("retrieving");
            return;
          case "retrieved_candidates":
            setByStrategy(event.by_strategy);
            // Show partial source ids early so skeletons can resolve.
            setPartialSourceIds(event.merged_ids.slice(0, 8));
            setPhase("reranking");
            return;
          case "reranked":
            setPartialSourceIds(event.source_ids);
            setPhase("synthesizing");
            return;
          case "retrieved":
            // Final retrieval signal — sources are locked in for synthesis.
            setPartialSourceIds(event.source_ids);
            setPhase("synthesizing");
            return;
          case "verified":
            setVerified({ unsupported: event.unsupported_claims });
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
            // Persist as recent (de-dup by query).
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
        if (phase !== "error") {
          setError(String(e));
          setPhase("error");
        }
      }
    },
    [corpusId, phase],
  );

  function applyStarter(s: string) {
    setQuery(s);
    submit(s);
  }

  function restoreRecent(e: RecentEntry) {
    setQuery(e.query);
    // Cache should hit and round-trip in ~10ms; just re-submit.
    submit(e.query);
  }

  function clearRecent() {
    setRecent([]);
    saveRecent(corpusId, []);
  }

  // Derived: which numeric citations the answer actually mentions.
  const cited = useMemo(
    () => (done ? citedIndices(done.answer) : new Set<number>()),
    [done],
  );

  // ── Empty: no corpus selected ────────────────────────────────────────────
  if (!corpus) {
    return (
      <EmptyState
        icon={Sparkles}
        title="NO CORPUS"
        hint="Add or select a project on the Projects tab to start asking questions."
      />
    );
  }

  return (
    <div className="@container/page flex h-full gap-4 min-h-0">
      {/* LEFT: prompt + answer column */}
      <div className="flex-1 min-w-0 flex flex-col gap-4 min-h-0">
        <Header corpus={corpus} health={health} />

        <Omnibar
          inputRef={inputRef}
          query={query}
          onChange={setQuery}
          onSubmit={() => submit(query)}
          loading={
            phase === "analyzing" ||
            phase === "retrieving" ||
            phase === "reranking" ||
            phase === "synthesizing" ||
            phase === "verifying"
          }
          disabled={!health?.available}
        />

        {/* Phase rail — visible during loading and immediately after. */}
        {phase !== "idle" && phase !== "error" && (
          <PhaseRail
            phase={phase}
            cached={done?.cached ?? false}
            verified={verified !== null}
          />
        )}

        {/* Sub-question strip — visible from `analyzing` onward. */}
        {analysis && analysis.sub_questions.length > 0 && phase !== "error" && (
          <SubQuestionStrip
            subQuestions={analysis.sub_questions}
            symbolHints={analysis.symbol_hints}
            bridgeRelevant={analysis.bridge_relevant}
            byStrategy={byStrategy}
          />
        )}

        {/* Body */}
        <div className="flex-1 min-h-0 overflow-y-auto">
          {phase === "idle" && (
            <Starters onApply={applyStarter} disabled={!health?.available} />
          )}

          {phase === "error" && error && (
            <ErrorCard
              message={error}
              onRetry={() => submit(query)}
              health={health}
            />
          )}

          {(phase === "analyzing" ||
            phase === "retrieving" ||
            phase === "reranking" ||
            phase === "synthesizing" ||
            phase === "verifying") && (
            <LoadingBody
              partialSourceIds={partialSourceIds}
              corpusId={corpusId}
              corpus={corpus}
              phase={phase}
            />
          )}

          {phase === "done" && done && (
            <ResultBody
              entry={done}
              corpusId={corpusId}
              corpus={corpus}
              cited={cited}
              verified={verified}
            />
          )}
        </div>
      </div>

      {/* RIGHT: recent answers strip — only on wider viewports. */}
      <aside className="hidden @min-[1280px]/page:flex w-[280px] shrink-0 flex-col gap-3 min-h-0">
        <RecentStrip
          entries={recent}
          activeQuery={done?.query ?? query}
          onPick={restoreRecent}
          onClear={clearRecent}
        />
      </aside>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Sub-components

function Header({
  corpus,
  health,
}: {
  corpus: CorpusInfo;
  health: InferenceHealth | null;
}) {
  return (
    <div className="flex items-center justify-between gap-3 shrink-0">
      <div className="flex items-baseline gap-3 min-w-0">
        <Sparkles className="h-4 w-4 text-accent shrink-0" strokeWidth={2.5} />
        <h1 className="font-serif text-2xl font-bold text-text leading-none">
          Ask
        </h1>
        <span className="font-mono text-xs uppercase tracking-[0.05em] text-text-dim truncate">
          {corpusLabel(corpus)}
        </span>
      </div>
      {health && !health.available && (
        <span className="inline-flex items-center gap-1.5 border border-danger bg-surface px-2 py-1 font-mono text-[0.6875rem] uppercase tracking-[0.05em] text-danger">
          <Terminal className="h-3 w-3" strokeWidth={2.5} />
          inference unavailable
        </span>
      )}
    </div>
  );
}

function SubQuestionStrip({
  subQuestions,
  symbolHints,
  bridgeRelevant,
  byStrategy,
}: {
  subQuestions: string[];
  symbolHints: string[];
  bridgeRelevant: boolean;
  byStrategy: Record<string, number>;
}) {
  // Total candidates retrieved across all strategies for this run.
  const totalCandidates = Object.values(byStrategy).reduce((a, b) => a + b, 0);

  return (
    <div className="flex flex-col gap-2 border border-border-soft bg-surface px-3 py-2 shrink-0">
      <div className="flex items-center gap-2">
        <span className="font-mono text-[0.6875rem] uppercase tracking-[0.05em] text-text-dim">
          Plan
        </span>
        {totalCandidates > 0 && (
          <span className="font-mono text-[0.6875rem] tabular-nums text-text-dim">
            · {totalCandidates} candidate{totalCandidates === 1 ? "" : "s"}
          </span>
        )}
        <span className="flex-1" />
        {bridgeRelevant && (
          <span
            className="inline-flex items-center gap-1 border border-border-soft bg-surface-overlay px-1.5 py-0.5 font-mono text-[0.625rem] uppercase tracking-[0.05em] text-text-muted"
            title="The query analysis flagged this as involving cross-language boundaries; bridge index was searched."
          >
            bridge
          </span>
        )}
      </div>
      <ol className="flex flex-col gap-1">
        {subQuestions.map((sq, i) => (
          <li key={`${i}-${sq}`} className="flex items-start gap-2">
            <span className="font-mono text-[0.6875rem] font-bold text-accent tabular-nums shrink-0 mt-0.5">
              {i + 1}.
            </span>
            <span className="font-sans text-xs text-text leading-snug">
              {sq}
            </span>
          </li>
        ))}
      </ol>
      {symbolHints.length > 0 && (
        <div className="flex items-center gap-1.5 flex-wrap">
          <span className="font-mono text-[0.625rem] uppercase tracking-[0.05em] text-text-dim">
            Symbols
          </span>
          {symbolHints.slice(0, 8).map((h) => (
            <span
              key={h}
              className="inline-flex items-center border border-border-soft bg-surface-overlay px-1.5 py-px font-mono text-[0.6875rem] text-text-muted"
              style={{ borderRadius: "var(--radius-pill)" }}
            >
              {h}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

function Omnibar({
  inputRef,
  query,
  onChange,
  onSubmit,
  loading,
  disabled,
}: {
  inputRef: React.RefObject<HTMLTextAreaElement | null>;
  query: string;
  onChange: (s: string) => void;
  onSubmit: () => void;
  loading: boolean;
  disabled: boolean;
}) {
  function onKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    // ⌘⏎ / Ctrl+⏎ always submits. Plain ⏎ submits unless Shift is held.
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey || !e.shiftKey)) {
      e.preventDefault();
      onSubmit();
    }
  }

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        onSubmit();
      }}
      className="flex flex-col gap-2 shrink-0"
    >
      <div className="flex items-start gap-2">
        <span
          className="font-mono text-2xl font-bold text-accent leading-none pt-2 shrink-0"
          aria-hidden="true"
        >
          ?
        </span>
        <textarea
          ref={inputRef}
          value={query}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder={
            disabled
              ? "Install the Claude CLI to enable Ask…"
              : "Ask anything about the codebase. ⏎ to submit, ⇧⏎ for newline."
          }
          rows={2}
          autoFocus
          spellCheck={false}
          disabled={disabled}
          className={cn(
            "min-h-[3.25rem] flex-1 border border-border-soft bg-surface px-3 py-2",
            "text-base font-sans text-text placeholder:text-text-dim",
            "placeholder:normal-case focus:outline-none focus:border-accent",
            "transition-none resize-none",
            "disabled:opacity-60 disabled:cursor-not-allowed",
          )}
        />
        <Button
          type="submit"
          size="lg"
          disabled={loading || disabled || !query.trim()}
        >
          {loading ? (
            <Loader2 className="h-4 w-4 animate-spin" strokeWidth={2.5} />
          ) : (
            <ArrowRight className="h-4 w-4" strokeWidth={2.5} />
          )}
          {loading ? "Asking" : "Ask"}
          <kbd
            className="ml-1 border border-border-soft bg-surface-overlay px-1 text-[0.6875rem] font-mono text-text-dim hidden sm:inline-flex"
            style={{ borderRadius: "var(--radius-pill)" }}
          >
            ⏎
          </kbd>
        </Button>
      </div>
    </form>
  );
}

type RailPhase =
  | "analyzing"
  | "retrieving"
  | "reranking"
  | "synthesizing"
  | "verifying"
  | "done";

function PhaseRail({
  phase,
  cached,
  verified,
}: {
  phase: RailPhase;
  cached: boolean;
  verified: boolean;
}) {
  // Cached answers skip almost everything — collapse the rail.
  const stages = cached
    ? [
        { id: "synthesizing", label: "Cache hit" },
        { id: "done", label: "Done" },
      ]
    : [
        { id: "analyzing", label: "Analyzing query" },
        { id: "retrieving", label: "Retrieving" },
        { id: "reranking", label: "Reranking" },
        { id: "synthesizing", label: "Synthesizing" },
        ...(verified
          ? [{ id: "verifying", label: "Verifying" }]
          : []),
        { id: "done", label: "Done" },
      ];
  const currentIdx = stages.findIndex((s) => s.id === phase);

  return (
    <div className="flex items-stretch gap-0 border border-border-soft bg-surface shrink-0 overflow-x-auto">
      {stages.map((s, i) => {
        const active = i === currentIdx;
        const past = i < currentIdx;
        return (
          <div
            key={s.id}
            className={cn(
              "flex-1 min-w-[7.5rem] flex items-center gap-2.5 px-3 py-2",
              i > 0 && "border-l border-border-soft",
              active && "bg-surface-overlay",
            )}
          >
            <div
              className={cn(
                "h-2 w-2 shrink-0 transition-none",
                active && "bg-accent animate-pulse",
                past && "bg-text-muted",
                !active && !past && "bg-border",
              )}
            />
            <span
              className={cn(
                "font-mono text-[0.6875rem] uppercase tracking-[0.05em] truncate",
                active ? "text-text" : "text-text-dim",
              )}
            >
              {String(i + 1).padStart(2, "0")} · {s.label}
            </span>
          </div>
        );
      })}
    </div>
  );
}

function Starters({
  onApply,
  disabled,
}: {
  onApply: (s: string) => void;
  disabled: boolean;
}) {
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <span className="font-mono text-[0.6875rem] uppercase tracking-[0.05em] text-text-dim">
          Starter questions
        </span>
        <span className="flex-1 h-px bg-border-soft" />
      </div>
      <div className="grid grid-cols-1 @min-[680px]/page:grid-cols-2 gap-2">
        {STARTERS.map((s) => (
          <button
            key={s}
            onClick={() => onApply(s)}
            disabled={disabled}
            className={cn(
              "group flex items-start gap-2.5 border border-border-soft bg-surface",
              "px-3 py-2.5 text-left",
              "hover:border-accent hover:bg-surface-overlay",
              "disabled:opacity-50 disabled:hover:border-border-soft disabled:hover:bg-surface",
              "cursor-pointer disabled:cursor-not-allowed transition-none",
            )}
          >
            <span className="font-mono text-xs font-bold text-accent group-hover:text-accent shrink-0 mt-0.5">
              ?
            </span>
            <span className="font-sans text-sm text-text leading-snug">
              {s}
            </span>
          </button>
        ))}
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
    <div className="border border-danger bg-surface p-4 flex items-start gap-3 border-l-2">
      <AlertTriangle
        className="h-4 w-4 text-danger shrink-0 mt-0.5"
        strokeWidth={2}
      />
      <div className="flex-1 min-w-0">
        <p className="font-serif text-base font-bold text-danger">
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

function LoadingBody({
  partialSourceIds,
  corpusId,
  corpus,
  phase,
}: {
  partialSourceIds: string[];
  corpusId: string;
  corpus: CorpusInfo | null;
  phase:
    | "analyzing"
    | "retrieving"
    | "reranking"
    | "synthesizing"
    | "verifying";
}) {
  const skeletonCount = partialSourceIds.length || 4;
  const label =
    phase === "analyzing"
      ? "Decomposing the question…"
      : phase === "retrieving"
        ? "Searching the index across multiple strategies…"
        : phase === "reranking"
          ? "Reranking candidates by relevance…"
          : phase === "synthesizing"
            ? "Synthesizing answer with citations…"
            : "Verifying claims against sources…";
  return (
    <div className="flex flex-col gap-4">
      {/* Skeleton answer card with shimmer */}
      <Card className="space-y-3">
        <div className="flex items-center gap-2">
          <Loader2
            className="h-3.5 w-3.5 text-accent animate-spin"
            strokeWidth={2.5}
          />
          <span className="font-mono text-[0.6875rem] uppercase tracking-[0.05em] text-text-dim">
            {label}
          </span>
        </div>
        <div className="space-y-2">
          <Shimmer width="92%" />
          <Shimmer width="76%" />
          <Shimmer width="84%" />
          <Shimmer width="60%" />
        </div>
      </Card>

      {/* Source cards: real once retrieved, ghost otherwise. */}
      <div className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <span className="font-mono text-[0.6875rem] uppercase tracking-[0.05em] text-text-dim">
            Sources
          </span>
          {partialSourceIds.length > 0 && (
            <span className="font-mono text-[0.6875rem] tabular-nums text-text-dim">
              ({partialSourceIds.length})
            </span>
          )}
          <span className="flex-1 h-px bg-border-soft" />
        </div>
        {partialSourceIds.length > 0
          ? partialSourceIds.map((id, i) => (
              <SourceRow
                key={id}
                index={i + 1}
                contentId={id}
                corpusId={corpusId}
                corpus={corpus}
                cited={true}
                pending
              />
            ))
          : Array.from({ length: skeletonCount }).map((_, i) => (
              <Card key={i} sunken className="py-2.5">
                <Shimmer width="50%" />
              </Card>
            ))}
      </div>
    </div>
  );
}

function ResultBody({
  entry,
  corpusId,
  corpus,
  cited,
  verified,
}: {
  entry: RecentEntry;
  corpusId: string;
  corpus: CorpusInfo | null;
  cited: Set<number>;
  verified: { unsupported: string[] } | null;
}) {
  const [copied, setCopied] = useState(false);

  function copy() {
    navigator.clipboard
      .writeText(entry.answer)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {
        /* clipboard unavailable */
      });
  }

  return (
    <div className="flex flex-col gap-4">
      {/* Answer card */}
      <Card className="space-y-3">
        {/* Meta strip */}
        <div className="flex flex-wrap items-center gap-2 border-b border-border-soft pb-2 text-[0.6875rem] font-mono uppercase tracking-[0.05em]">
          {entry.cached ? (
            <span className="inline-flex items-center gap-1 border border-accent bg-surface-overlay px-1.5 py-0.5 text-accent">
              <Zap className="h-3 w-3" strokeWidth={2.5} />
              cached
            </span>
          ) : (
            <span className="inline-flex items-center gap-1 border border-border-soft bg-surface-overlay px-1.5 py-0.5 text-text-muted">
              fresh
            </span>
          )}
          {verified !== null && verified.unsupported.length === 0 && (
            <span className="inline-flex items-center gap-1 border border-accent bg-surface-overlay px-1.5 py-0.5 text-accent">
              <Check className="h-3 w-3" strokeWidth={2.5} />
              verified
            </span>
          )}
          {verified !== null && verified.unsupported.length > 0 && (
            <span
              className="inline-flex items-center gap-1 border border-danger bg-surface-overlay px-1.5 py-0.5 text-danger"
              title={`${verified.unsupported.length} unsupported claim(s) flagged`}
            >
              <AlertTriangle className="h-3 w-3" strokeWidth={2.5} />
              {verified.unsupported.length} flagged
            </span>
          )}
          {entry.model && (
            <span className="text-text-dim">model · {entry.model}</span>
          )}
          <span className="text-text-dim tabular-nums">
            {formatDuration(entry.elapsed_ms)}
          </span>
          <span className="text-text-dim tabular-nums">
            {entry.source_ids.length} source
            {entry.source_ids.length === 1 ? "" : "s"}
          </span>
          <span className="flex-1" />
          <button
            onClick={copy}
            className="inline-flex items-center gap-1 border border-border-soft bg-surface px-1.5 py-0.5 text-text-muted hover:text-text hover:border-border cursor-pointer transition-none"
          >
            {copied ? (
              <Check className="h-3 w-3" strokeWidth={2.5} />
            ) : (
              <Copy className="h-3 w-3" strokeWidth={2.5} />
            )}
            {copied ? "copied" : "copy"}
          </button>
        </div>

        <Answer
          answer={entry.answer}
          sourceIds={entry.source_ids}
          corpusId={corpusId}
        />
      </Card>

      {/* Sources panel — always shown, complement to inline chips. */}
      {entry.source_ids.length > 0 && (
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <span className="font-mono text-[0.6875rem] uppercase tracking-[0.05em] text-text-dim">
              Sources
            </span>
            <span className="font-mono text-[0.6875rem] tabular-nums text-text-dim">
              ({entry.source_ids.length})
            </span>
            <span className="flex-1 h-px bg-border-soft" />
            {cited.size > 0 && cited.size < entry.source_ids.length && (
              <span className="font-mono text-[0.6875rem] uppercase tracking-[0.05em] text-text-dim">
                {cited.size} cited
              </span>
            )}
          </div>
          {entry.source_ids.map((id, i) => (
            <SourceRow
              key={id}
              index={i + 1}
              contentId={id}
              corpusId={corpusId}
              corpus={corpus}
              cited={cited.size === 0 || cited.has(i + 1)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Markdown answer with inline citation chips

function Answer({
  answer,
  sourceIds,
  corpusId,
}: {
  answer: string;
  sourceIds: string[];
  corpusId: string;
}) {
  const { openEntity } = useEntityPanel();

  const transformed = useMemo(
    () => injectCitationMarkers(answer),
    [answer],
  );

  function openCitation(n: number) {
    const id = sourceIds[n - 1];
    if (!id) return;
    void resolveAndOpen(corpusId, id, openEntity);
  }

  return (
    <div className="ask-answer font-serif text-[0.9375rem] leading-relaxed text-text">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          // Inline code: monospaced, surface-overlay background.
          code({ className, children, ...props }) {
            const inline = !className?.includes("language-");
            if (inline) {
              return (
                <code
                  className="border border-border-soft bg-surface-overlay px-1 py-px text-[0.875em] font-mono text-text"
                  {...props}
                >
                  {children}
                </code>
              );
            }
            return (
              <code className={cn(className, "font-mono")} {...props}>
                {children}
              </code>
            );
          },
          pre({ children, ...props }) {
            return (
              <pre
                className="border border-border-soft bg-surface-overlay p-3 my-3 overflow-x-auto text-[0.8125rem] font-mono text-text"
                {...props}
              >
                {children}
              </pre>
            );
          },
          a({ children, href, ...props }) {
            return (
              <a
                href={href}
                className="text-accent underline underline-offset-2 hover:text-accent-hover"
                target="_blank"
                rel="noreferrer"
                {...props}
              >
                {children}
              </a>
            );
          },
          ul({ children }) {
            return (
              <ul className="list-disc pl-5 my-2 space-y-1">{children}</ul>
            );
          },
          ol({ children }) {
            return (
              <ol className="list-decimal pl-5 my-2 space-y-1">{children}</ol>
            );
          },
          h1({ children }) {
            return <h2 className="text-xl font-bold mt-4 mb-2">{children}</h2>;
          },
          h2({ children }) {
            return <h3 className="text-lg font-bold mt-3 mb-1.5">{children}</h3>;
          },
          h3({ children }) {
            return (
              <h4 className="text-base font-bold mt-3 mb-1">{children}</h4>
            );
          },
          p({ children }) {
            return <p className="my-2">{renderWithCitations(children, openCitation)}</p>;
          },
          li({ children }) {
            return <li>{renderWithCitations(children, openCitation)}</li>;
          },
        }}
      >
        {transformed}
      </ReactMarkdown>
    </div>
  );
}

/** Replace `[N]` and `[N, M]` with sentinel markers our renderer rewrites
 *  into clickable chips. We keep markdown semantics for everything else. */
function injectCitationMarkers(text: string): string {
  // The sentinel uses a U+2042 character pair so it won't collide with
  // anything markdown might generate; the recursive renderer below splits
  // text nodes on it.
  return text.replace(
    /\[(\d+(?:\s*,\s*\d+)*)\]/g,
    (_m, group) => `⁂${group}⁂`,
  );
}

/** Walk children produced by react-markdown and rewrite text nodes that
 *  contain our sentinel into a mix of plain text + citation chips. */
function renderWithCitations(
  children: ReactNode,
  open: (n: number) => void,
): ReactNode {
  if (typeof children === "string") {
    return splitOnSentinel(children, open);
  }
  if (Array.isArray(children)) {
    return children.map((c, i) => (
      <span key={i}>{renderWithCitations(c, open)}</span>
    ));
  }
  return children;
}

function splitOnSentinel(text: string, open: (n: number) => void): ReactNode {
  const parts = text.split(/⁂([\d, ]+)⁂/);
  if (parts.length === 1) return text;
  return parts.map((part, i) => {
    // Even indices are literal text; odd indices are the captured groups.
    if (i % 2 === 0) return part;
    const numbers = part
      .split(",")
      .map((s) => parseInt(s.trim(), 10))
      .filter((n) => Number.isFinite(n) && n > 0);
    return (
      <span key={i} className="inline-flex items-baseline gap-0.5 mx-0.5">
        {numbers.map((n) => (
          <CitationChip key={n} n={n} onClick={() => open(n)} />
        ))}
      </span>
    );
  });
}

function CitationChip({ n, onClick }: { n: number; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      title={`Open source [${n}]`}
      className={cn(
        "inline-flex items-center justify-center align-baseline",
        "border border-accent bg-surface text-accent",
        "px-1 min-w-[1.25rem] h-[1.125rem] -translate-y-[1px]",
        "font-mono text-[0.6875rem] font-bold tabular-nums leading-none",
        "hover:bg-accent hover:text-[var(--color-accent-fg-on)]",
        "cursor-pointer transition-none",
      )}
      style={{ borderRadius: "var(--radius-pill)" }}
    >
      {n}
    </button>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Source row (cited or not)

function SourceRow({
  index,
  contentId,
  corpusId,
  corpus,
  cited,
  pending = false,
}: {
  index: number;
  contentId: string;
  corpusId: string;
  corpus: CorpusInfo | null;
  cited: boolean;
  pending?: boolean;
}) {
  const { openEntity } = useEntityPanel();
  const [excerpt, setExcerpt] = useState<string | null>(null);
  const [headingPath, setHeadingPath] = useState<string[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    setExcerpt(null);
    setHeadingPath(null);
    fetchSourcePreview(corpusId, contentId).then((p) => {
      if (cancelled) return;
      setExcerpt(p.excerpt);
      setHeadingPath(p.headingPath);
    });
    return () => {
      cancelled = true;
    };
  }, [corpusId, contentId]);

  function open() {
    void resolveAndOpen(corpusId, contentId, openEntity);
  }

  const filePath = filePathFromContentId(contentId);
  const fileTag = corpusRelative(filePath, corpus);
  const label = sourceLabel(contentId, headingPath ?? undefined);

  return (
    <button
      onClick={open}
      className={cn(
        "group flex items-start gap-3 border bg-surface p-2.5 text-left",
        "cursor-pointer transition-none",
        cited
          ? "border-border-soft hover:border-accent hover:bg-surface-overlay"
          : "border-border-soft opacity-60 hover:opacity-100 hover:border-border",
        pending && "animate-pulse",
      )}
    >
      <span
        className={cn(
          "inline-flex items-center justify-center shrink-0 mt-0.5",
          "border h-5 min-w-[1.25rem] px-1",
          "font-mono text-[0.6875rem] font-bold tabular-nums leading-none",
          cited
            ? "border-accent bg-surface text-accent"
            : "border-border-soft bg-surface text-text-dim",
        )}
        style={{ borderRadius: "var(--radius-pill)" }}
      >
        {index}
      </span>
      <div className="flex-1 min-w-0">
        <div className="flex items-baseline gap-2 min-w-0">
          <span className="font-sans text-sm font-medium text-text truncate">
            {label}
          </span>
          {fileTag && fileTag !== label && (
            <span className="font-mono text-[0.6875rem] text-text-dim truncate">
              {fileTag}
            </span>
          )}
        </div>
        {excerpt && (
          <p className="font-mono text-xs text-text-muted mt-1 line-clamp-2 break-words">
            {excerpt}
          </p>
        )}
      </div>
      <ExternalLink
        className="h-3.5 w-3.5 text-text-dim group-hover:text-accent shrink-0 mt-1"
        strokeWidth={2}
      />
    </button>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Recent strip

function RecentStrip({
  entries,
  activeQuery,
  onPick,
  onClear,
}: {
  entries: RecentEntry[];
  activeQuery: string;
  onPick: (e: RecentEntry) => void;
  onClear: () => void;
}) {
  return (
    <div className="flex flex-col gap-2 min-h-0">
      <div className="flex items-center gap-2 shrink-0">
        <History className="h-3.5 w-3.5 text-text-dim" strokeWidth={2} />
        <span className="font-mono text-[0.6875rem] uppercase tracking-[0.05em] text-text-dim">
          Recent
        </span>
        <span className="flex-1" />
        {entries.length > 0 && (
          <button
            onClick={onClear}
            title="Clear recent"
            className="text-text-dim hover:text-text cursor-pointer transition-none"
          >
            <X className="h-3.5 w-3.5" strokeWidth={2} />
          </button>
        )}
      </div>
      <div className="flex-1 min-h-0 overflow-y-auto flex flex-col gap-1.5">
        {entries.length === 0 ? (
          <p className="font-mono text-[0.6875rem] text-text-dim italic px-1">
            Your recent questions will appear here.
          </p>
        ) : (
          entries.map((e) => {
            const active =
              activeQuery.trim().toLowerCase() === e.query.trim().toLowerCase();
            return (
              <button
                key={`${e.ts}-${e.query}`}
                onClick={() => onPick(e)}
                className={cn(
                  "group flex flex-col items-start gap-0.5 border bg-surface p-2 text-left",
                  "cursor-pointer transition-none",
                  active
                    ? "border-accent bg-surface-overlay"
                    : "border-border-soft hover:border-border hover:bg-surface-overlay",
                )}
              >
                <div className="flex items-center gap-1.5 w-full">
                  {e.cached && (
                    <Zap
                      className="h-3 w-3 text-accent shrink-0"
                      strokeWidth={2.5}
                    />
                  )}
                  <span className="font-sans text-xs text-text truncate flex-1">
                    {e.query}
                  </span>
                </div>
                <span className="font-mono text-[0.625rem] text-text-dim tabular-nums">
                  {e.source_ids.length} src · {formatDuration(e.elapsed_ms)}
                </span>
              </button>
            );
          })
        )}
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers

function Shimmer({ width = "100%" }: { width?: string }) {
  return (
    <div
      className="h-3 bg-surface-overlay animate-pulse"
      style={{ width }}
      aria-hidden="true"
    />
  );
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

/** Best-effort cache for source previews so flipping between the loading
 *  state and the result state doesn't refetch the same section. */
const sourcePreviewCache = new Map<
  string,
  { excerpt: string | null; headingPath: string[] | null }
>();

async function fetchSourcePreview(
  corpusId: string,
  contentId: string,
): Promise<{ excerpt: string | null; headingPath: string[] | null }> {
  const cacheKey = `${corpusId}::${contentId}`;
  const cached = sourcePreviewCache.get(cacheKey);
  if (cached) return cached;

  // Symbol IDs (sym-prefix) → symbol_definition; otherwise → read_section.
  const isSymbol = contentId.startsWith("sym-");
  let result: { excerpt: string | null; headingPath: string[] | null } = {
    excerpt: null,
    headingPath: null,
  };
  try {
    if (isSymbol) {
      const def = await invoke<SymbolDefinitionDetail>("symbol_definition", {
        corpusId,
        symbolId: contentId,
      });
      result = {
        excerpt: shortExcerpt(def.source_context || def.signature || ""),
        headingPath:
          def.heading_path && def.heading_path.length > 0
            ? def.heading_path
            : [`${def.kind} ${def.name}`],
      };
    } else {
      const det = await invoke<SectionDetailOut>("read_section", {
        corpusId,
        sectionId: contentId,
      });
      result = {
        excerpt: shortExcerpt(det.text),
        headingPath: det.heading_path,
      };
    }
  } catch {
    /* leave result empty on failure */
  }
  sourcePreviewCache.set(cacheKey, result);
  return result;
}

function shortExcerpt(text: string): string {
  const trimmed = text.trim().replace(/\s+/g, " ");
  return trimmed.length > 220 ? trimmed.slice(0, 220) + "…" : trimmed;
}

/** Resolve a content_id to a full SearchResult/SymbolInfo and open the
 *  global EntityPanel. Used by both citation chip clicks and SourceRow. */
async function resolveAndOpen(
  corpusId: string,
  contentId: string,
  openEntity: ReturnType<typeof useEntityPanel>["openEntity"],
) {
  const isSymbol = contentId.startsWith("sym-");
  try {
    if (isSymbol) {
      const def = await invoke<SymbolDefinitionDetail>("symbol_definition", {
        corpusId,
        symbolId: contentId,
      });
      openEntity({
        kind: "symbol",
        corpusId,
        symbol: {
          id: def.id,
          name: def.name,
          kind: def.kind,
          file_path: def.file_path,
          visibility: def.visibility,
          signature: def.signature,
          doc_comment: def.doc_comment,
          module_path: "",
        },
      });
    } else {
      const det = await invoke<SectionDetailOut>("read_section", {
        corpusId,
        sectionId: contentId,
      });
      const result: SearchResult = {
        content_id: det.section_id,
        resolution: "section",
        score: 0,
        text: det.text,
        heading_path: det.heading_path,
      };
      openEntity({
        kind: "section",
        corpusId,
        result,
      });
    }
  } catch {
    /* swallow — citation just won't open */
  }
}
