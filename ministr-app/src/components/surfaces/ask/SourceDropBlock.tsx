/**
 * SourceDropBlock — a source dropped INTO the Ask thread (aaa-ask-citation-dropin).
 *
 * Opening a citation no longer only flashes the transient EntityPanel drawer:
 * it appends one of these as a first-class, persisted thread entry you can KEEP
 * while you build understanding. Distinct from the answer card (a left accent
 * rail + a "SOURCE" tag) so it reads as a kept reference, not another answer.
 *
 * Built from scratch on the v4 tokens + ui/ atoms (Card, CodeExcerpt); it
 * reuses only the shared source-preview fetch + the EntityPanel resolver, never
 * an assembled composition. The drawer open path is preserved via "Open ↗".
 */
import { useEffect, useState } from "react";
import { Bookmark, ExternalLink, X } from "lucide-react";

import type { CorpusInfo } from "../../../lib/types";
import { Card } from "../../ui/card";
import { CodeExcerpt } from "../../ui/code-excerpt";
import { useEntityPanel } from "../../../hooks/useEntityPanel";
import { basename, corpusRelative } from "../../../lib/path";
import { cn } from "../../../lib/utils";
import { resolveAndOpen } from "./AskAnswer";
import { fetchSourcePreview, filePathFromContentId } from "./internals";
import type { DroppedSource } from "./thread";

interface Props {
  source: DroppedSource;
  corpusId: string;
  corpus: CorpusInfo | null;
  /** Remove this kept source from the thread. */
  onRemove: () => void;
}

export function SourceDropBlock({ source, corpusId, corpus, onRemove }: Props) {
  const { openEntity } = useEntityPanel();
  const [excerpt, setExcerpt] = useState<string | null>(null);
  const [headingPath, setHeadingPath] = useState<string[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    setExcerpt(null);
    setHeadingPath(null);
    fetchSourcePreview(corpusId, source.contentId).then((p) => {
      if (cancelled) return;
      setExcerpt(p.excerpt);
      setHeadingPath(p.headingPath);
    });
    return () => {
      cancelled = true;
    };
  }, [corpusId, source.contentId]);

  const filePath = filePathFromContentId(source.contentId);
  const fileTag = corpusRelative(filePath, corpus);
  const label = sourceLabel(source.contentId, headingPath ?? undefined);

  function open() {
    void resolveAndOpen(corpusId, source.contentId, openEntity);
  }

  return (
    <div className="flex gap-3">
      {/* The kept-reference rail — mirrors AskQuestion's accent rule idiom but
          in the "info" tone so a pinned source reads apart from Q and A. */}
      <span
        className="mt-1 w-0.5 self-stretch shrink-0 rounded-full bg-info/60"
        aria-hidden
      />
      <Card className="flex-1 min-w-0 space-y-2.5">
        <div className="flex items-center gap-2 min-w-0">
          <Bookmark className="h-3.5 w-3.5 text-info shrink-0" strokeWidth={2.25} />
          <span className="font-mono text-mono-micro uppercase tracking-[0.08em] text-info">
            Source
          </span>
          {typeof source.n === "number" && (
            <span
              className={cn(
                "inline-flex items-center justify-center shrink-0",
                "border border-info/60 h-[1.125rem] min-w-[1.25rem] px-1 rounded-md",
                "font-mono text-mono-mini font-bold tabular-nums leading-none text-info",
              )}
            >
              {source.n}
            </span>
          )}
          <span className="font-sans text-sm font-medium text-text truncate">
            {label}
          </span>
          <span className="flex-1" />
          <button
            onClick={open}
            title="Open in inspector"
            aria-label="Open in inspector"
            className={cn(
              "inline-flex items-center gap-1 px-1.5 py-0.5 shrink-0 rounded-md cursor-pointer",
              "font-mono text-mono-mini font-semibold uppercase tracking-[0.08em]",
              "border border-border-soft bg-surface text-text-muted",
              "hover:text-text hover:border-border transition-colors duration-150",
            )}
          >
            <ExternalLink className="h-3 w-3" strokeWidth={2} />
            Open
          </button>
          <button
            onClick={onRemove}
            title="Remove from thread"
            aria-label="Remove source from thread"
            className={cn(
              "grid place-items-center h-6 w-6 shrink-0 rounded-md cursor-pointer",
              "text-text-dim hover:bg-surface-overlay hover:text-danger",
              "transition-colors duration-150",
            )}
          >
            <X className="h-3.5 w-3.5" strokeWidth={2} />
          </button>
        </div>

        {fileTag && fileTag !== label && (
          <p className="font-mono text-mono-mini text-text-dim truncate">
            {fileTag}
          </p>
        )}

        {excerpt ? (
          <CodeExcerpt code={excerpt} filename={filePath} maxLines={6} />
        ) : (
          <div className="space-y-1.5" aria-hidden>
            {[88, 72, 94, 64].map((w, i) => (
              <div
                key={i}
                className="h-2 ministr-skeleton"
                style={{ width: `${w}%` }}
              />
            ))}
          </div>
        )}
      </Card>
    </div>
  );
}

/** Heading-path label (file-basename segment trimmed when it duplicates the
 *  file) > basename of file. Mirrors AskAnswer's sourceLabel. */
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
