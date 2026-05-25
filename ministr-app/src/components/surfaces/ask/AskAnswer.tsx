import { useEffect, useMemo, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle,
  Check,
  Copy,
  ExternalLink,
} from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import type {
  CorpusInfo,
  SearchResult,
  SymbolDefinitionDetail,
} from "../../../lib/types";
import { Card } from "../../ui/card";
import { BrutalPin } from "../../ui/brutal-icons";
import { useEntityPanel } from "../../../hooks/useEntityPanel";
import { basename, corpusRelative } from "../../../lib/path";
import { cn } from "../../../lib/utils";
import { AskCitation } from "./AskCitation";
import {
  citedIndices,
  fetchSourcePreview,
  filePathFromContentId,
  formatDuration,
  type RecentEntry,
  type SectionDetailOut,
} from "./internals";

interface Props {
  entry: RecentEntry;
  corpusId: string;
  corpus: CorpusInfo | null;
  /** When true the parent surface is rendering the verification banner;
   *  the answer card hides any "checking sources…" mention. */
  verifiedUnsupported: string[] | null;
  pinned: boolean;
  onPin: () => void;
  onUnpin: () => void;
}

/**
 * Rendered Q&A card — markdown answer with inline citation chips, a
 * collapsible Sources panel, copy-to-clipboard, and a Pin button.
 *
 * Replaces `ResultBody` from the old `AskView`. Three deliberate
 * regressions vs the old surface:
 *   - The "fresh / verified / model" jargon strip is gone — `AskStatus`
 *     handles cache-hit messaging and the verification banner is parent-
 *     owned.
 *   - The "sources first" toggle is gone (was rarely used; if anyone
 *     misses it, restore as a preference).
 *   - Per-source pinning is gone; pinning is one-per-answer now.
 */
export function AskAnswer({
  entry,
  corpusId,
  corpus,
  verifiedUnsupported,
  pinned,
  onPin,
  onUnpin,
}: Props) {
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

  const cited = useMemo(() => citedIndices(entry.answer), [entry.answer]);

  return (
    <div className="flex flex-col gap-4">
      {verifiedUnsupported && verifiedUnsupported.length > 0 && (
        <UnsupportedBanner count={verifiedUnsupported.length} />
      )}

      <Card className="space-y-3">
        <div className="flex flex-wrap items-center gap-2 border-b border-border-soft pb-2">
          <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
            Answer
          </span>
          <span className="font-mono text-mono-mini text-text-dim tabular-nums">
            {formatDuration(entry.elapsed_ms)}
          </span>
          <span className="font-mono text-mono-mini text-text-dim tabular-nums">
            {entry.source_ids.length} source
            {entry.source_ids.length === 1 ? "" : "s"}
          </span>
          <span className="flex-1" />
          <button
            onClick={pinned ? onUnpin : onPin}
            title={pinned ? "Unpin this answer" : "Pin this answer"}
            className={cn(
              "inline-flex items-center gap-1 border px-1.5 py-0.5 cursor-pointer transition-colors duration-150 rounded-full",
              "font-mono text-mono-mini font-semibold uppercase tracking-[0.08em]",
              pinned
                ? "border-info bg-surface-overlay text-info"
                : "border-border-soft bg-surface text-text-muted hover:text-text hover:border-border",
            )}
          >
            <BrutalPin className="h-3 w-3" />
            {pinned ? "Pinned" : "Pin"}
          </button>
          <button
            onClick={copy}
            title={copied ? "Copied" : "Copy to clipboard"}
            className={cn(
              "inline-flex items-center gap-1 border px-1.5 py-0.5 cursor-pointer transition-colors duration-150 rounded-full",
              "font-mono text-mono-mini font-semibold uppercase tracking-[0.08em]",
              "border-border-soft bg-surface text-text-muted hover:text-text hover:border-border",
            )}
          >
            {copied ? (
              <Check className="h-3 w-3" strokeWidth={2.5} />
            ) : (
              <Copy className="h-3 w-3" strokeWidth={2.5} />
            )}
            {copied ? "Copied" : "Copy"}
          </button>
        </div>

        <Answer
          answer={entry.answer}
          sourceIds={entry.source_ids}
          corpusId={corpusId}
          pinned={pinned}
          onPinAnswer={pinned ? undefined : onPin}
        />
      </Card>

      {entry.source_ids.length > 0 && (
        <SourcesPanel
          sourceIds={entry.source_ids}
          cited={cited}
          corpusId={corpusId}
          corpus={corpus}
        />
      )}
    </div>
  );
}

function UnsupportedBanner({ count }: { count: number }) {
  return (
    <div className="flex items-start gap-2 rounded-lg border border-danger/50 bg-danger/10 px-3 py-2.5">
      <AlertTriangle
        className="h-4 w-4 text-danger shrink-0 mt-0.5"
        strokeWidth={2}
      />
      <div className="flex-1 min-w-0">
        <p className="font-sans text-sm font-medium text-text">
          {count} claim{count === 1 ? "" : "s"} not backed by sources
        </p>
        <p className="font-sans text-xs text-text-dim mt-0.5">
          Citation-checking flagged statements that don&apos;t appear in the
          retrieved excerpts. Open each source to verify before relying on it.
        </p>
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Markdown answer with inline citation chips.

function Answer({
  answer,
  sourceIds,
  corpusId,
  pinned,
  onPinAnswer,
}: {
  answer: string;
  sourceIds: string[];
  corpusId: string;
  pinned: boolean;
  onPinAnswer?: () => void;
}) {
  const { openEntity } = useEntityPanel();

  const transformed = useMemo(() => injectCitationMarkers(answer), [answer]);

  function openCitation(n: number) {
    const id = sourceIds[n - 1];
    if (!id) return;
    void resolveAndOpen(corpusId, id, openEntity);
  }

  return (
    <div className="ask-answer font-sans text-[0.9375rem] leading-relaxed text-text">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
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
            return (
              <h3 className="text-lg font-bold mt-3 mb-1.5">{children}</h3>
            );
          },
          h3({ children }) {
            return (
              <h4 className="text-base font-bold mt-3 mb-1">{children}</h4>
            );
          },
          p({ children }) {
            return (
              <p className="my-2">
                {renderWithCitations(
                  children,
                  openCitation,
                  sourceIds,
                  corpusId,
                  pinned,
                  onPinAnswer,
                )}
              </p>
            );
          },
          li({ children }) {
            return (
              <li>
                {renderWithCitations(
                  children,
                  openCitation,
                  sourceIds,
                  corpusId,
                  pinned,
                  onPinAnswer,
                )}
              </li>
            );
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
  return text.replace(
    /\[(\d+(?:\s*,\s*\d+)*)\]/g,
    (_m, group) => `⁂${group}⁂`,
  );
}

function renderWithCitations(
  children: ReactNode,
  open: (n: number) => void,
  sourceIds: string[],
  corpusId: string,
  pinned: boolean,
  onPinAnswer?: () => void,
): ReactNode {
  if (typeof children === "string") {
    return splitOnSentinel(
      children,
      open,
      sourceIds,
      corpusId,
      pinned,
      onPinAnswer,
    );
  }
  if (Array.isArray(children)) {
    return children.map((c, i) => (
      <span key={i}>
        {renderWithCitations(c, open, sourceIds, corpusId, pinned, onPinAnswer)}
      </span>
    ));
  }
  return children;
}

function splitOnSentinel(
  text: string,
  open: (n: number) => void,
  sourceIds: string[],
  corpusId: string,
  pinned: boolean,
  onPinAnswer?: () => void,
): ReactNode {
  const parts = text.split(/⁂([\d, ]+)⁂/);
  if (parts.length === 1) return text;
  return parts.map((part, i) => {
    if (i % 2 === 0) return part;
    const numbers = part
      .split(",")
      .map((s) => parseInt(s.trim(), 10))
      .filter((n) => Number.isFinite(n) && n > 0);
    return (
      <span key={i} className="inline-flex items-baseline gap-0.5 mx-0.5">
        {numbers.map((n) => {
          const sourceId = sourceIds[n - 1];
          return (
            <AskCitation
              key={n}
              n={n}
              sourceId={sourceId}
              corpusId={corpusId}
              pinned={pinned}
              onPinAnswer={onPinAnswer}
              onOpen={(num) => open(num)}
            />
          );
        })}
      </span>
    );
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Sources list — one row per source.

function SourcesPanel({
  sourceIds,
  cited,
  corpusId,
  corpus,
}: {
  sourceIds: string[];
  cited: Set<number>;
  corpusId: string;
  corpus: CorpusInfo | null;
}) {
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
          Sources
        </span>
        <span className="font-mono text-mono-mini tabular-nums text-text-dim">
          ({sourceIds.length})
        </span>
        <span className="flex-1 h-px bg-border-soft" />
        {cited.size > 0 && cited.size < sourceIds.length && (
          <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
            {cited.size} cited
          </span>
        )}
      </div>
      {sourceIds.map((id, i) => (
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
  );
}

function SourceRow({
  index,
  contentId,
  corpusId,
  corpus,
  cited,
}: {
  index: number;
  contentId: string;
  corpusId: string;
  corpus: CorpusInfo | null;
  cited: boolean;
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
    <div
      onClick={open}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          open();
        }
      }}
      className={cn(
        "group flex items-start gap-3 rounded-lg border p-2.5 text-left",
        "cursor-pointer transition-colors duration-150",
        cited
          ? "border-border bg-surface hover:border-accent hover:bg-surface-overlay"
          : "border-border bg-surface opacity-60 hover:opacity-100 hover:border-border-hover",
      )}
    >
      <span
        className={cn(
          "inline-flex items-center justify-center shrink-0 mt-0.5",
          "border h-5 min-w-[1.25rem] px-1",
          "font-mono text-mono-mini font-bold tabular-nums leading-none",
          cited
            ? "border-accent bg-surface text-accent"
            : "border-border-soft bg-surface text-text-dim",
          "rounded-md",
        )}
      >
        {index}
      </span>
      <div className="flex-1 min-w-0">
        <div className="flex items-baseline gap-2 min-w-0">
          <span className="font-sans text-sm font-medium text-text truncate">
            {label}
          </span>
          {fileTag && fileTag !== label && (
            <span className="font-mono text-mono-mini text-text-dim truncate">
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
    </div>
  );
}

/** Short label for a source: heading path (with the file-basename
 *  segment trimmed when it duplicates the file tag) > basename of file. */
function sourceLabel(id: string, headingPath?: string[]): string {
  const file = filePathFromContentId(id);
  const fileBase = basename(file);
  const fileStem = fileBase.replace(/\.[^.]+$/, "");
  if (headingPath && headingPath.length > 0) {
    const trimmed =
      headingPath[0] === fileBase || headingPath[0] === fileStem
        ? headingPath.slice(1)
        : headingPath;
    if (trimmed.length > 0) return trimmed.join(" › ");
  }
  return fileBase || id;
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
