import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useEntityPanel, type Entity } from "../../hooks/useEntityPanel";
import { EntityRow } from "./EntityRow";
import {
  EntitySection,
  EntitySectionEmpty,
  EntitySectionLoading,
} from "./EntitySection";
import { ChevronRight, FileText } from "@/components/ui/icons";
import type { CoherenceEvent, SearchResult, SymbolInfo } from "../../lib/types";
import { basename } from "../../lib/path";

interface Props {
  entity: Extract<Entity, { kind: "section" }>;
}

/** Best-effort: extract the file path from a content_id like
 *  `d:/code/ministr/ministr-core/src/storage.rs#root:c0` */
function filePathFromContentId(contentId: string): string | null {
  // Strip leading `sym-` (symbol stub IDs) and any `#...` suffix.
  const noPrefix = contentId.replace(/^sym-/, "");
  const hashIdx = noPrefix.indexOf("#");
  const candidate = hashIdx >= 0 ? noPrefix.slice(0, hashIdx) : noPrefix;
  if (!candidate.includes("/") && !candidate.includes("\\")) return null;
  return candidate;
}

/** A human title for the section: the deepest heading segment, else the file
 *  basename, else the raw id. Keeps the identity legible instead of a content_id. */
function sectionTitle(result: SearchResult): string {
  const hp = result.heading_path;
  if (hp.length > 0) return hp[hp.length - 1];
  const fp = filePathFromContentId(result.content_id);
  if (fp) return basename(fp);
  return result.content_id;
}

export function SectionView({ entity }: Props) {
  const { corpusId, result } = entity;
  const { openEntity } = useEntityPanel();
  const filePath = filePathFromContentId(result.content_id);

  const [symbols, setSymbols] = useState<SymbolInfo[] | null>(null);
  const [changes, setChanges] = useState<CoherenceEvent[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    setSymbols(null);
    setChanges(null);

    const symbolsP = filePath
      ? invoke<SymbolInfo[]>("search_symbols", {
          corpusId,
          query: "",
          kind: null,
          filePath,
        })
      : Promise.resolve([] as SymbolInfo[]);

    // Same candidate-window pattern as SessionView/FileView/CorpusView:
    // pull a generous slice and filter client-side. Keeps section-scoped
    // history visible on busy multi-file corpora until the daemon learns
    // server-side file_path filtering.
    const changesP = invoke<CoherenceEvent[]>("recent_coherence_events", {
      limit: 500,
      sinceMs: null,
    });

    Promise.allSettled([symbolsP, changesP]).then(([s, c]) => {
      if (cancelled) return;
      setSymbols(s.status === "fulfilled" ? s.value : []);
      // When the content_id has no embedded file path (stub symbol ids,
      // legacy claim ids, etc.), there is no meaningful "Recent changes
      // for this section" feed to show — fall through to an empty list
      // so the §4 panel renders its empty state, never the unfiltered
      // global feed for unrelated files.
      //
      // Use full-path equality (after slash normalization), not
      // endsWith(): two corpus roots ending in the same relative path
      // (e.g. apps/a/src/lib.rs vs apps/b/src/lib.rs) would otherwise
      // cross-attribute each other's events.
      setChanges(
        c.status === "fulfilled" && filePath
          ? c.value.filter(
              (e) =>
                e.path.replace(/\\/g, "/") === filePath.replace(/\\/g, "/"),
            )
          : [],
      );
    });
    return () => {
      cancelled = true;
    };
  }, [corpusId, filePath, result.content_id]);

  return (
    <div className="flex flex-col gap-4">
      {/* §1 Overview — a command-deck source-identity header: a document
          medallion + the human title + file path + a relevance vital/meter,
          with the raw content_id kept but de-emphasized. */}
      <EntitySection chapter={1} title="Overview">
        <div className="space-y-3 px-3 py-3">
          <div className="flex items-start gap-3">
            {/* Quiet accent medallion — a source isn't "live", so no glow. */}
            <span
              aria-hidden
              className="grid h-11 w-11 shrink-0 place-items-center rounded-xl border border-accent/40 bg-surface-overlay text-accent"
            >
              <FileText className="h-[18px] w-[18px]" strokeWidth={2} />
            </span>
            <div className="min-w-0 flex-1">
              <p className="break-words text-[15px] font-semibold leading-tight text-text">
                {sectionTitle(result)}
              </p>
              {filePath && (
                <p className="mt-0.5 truncate font-mono text-mono-mini text-text-dim">
                  {filePath}
                </p>
              )}
            </div>
            {/* Relevance vital — tone stays off the number (AA). */}
            <div className="shrink-0 text-right">
              <p className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
                match
              </p>
              <p className="font-mono text-base font-semibold tabular-nums text-text">
                {(result.score * 100).toFixed(0)}%
              </p>
            </div>
          </div>

          {/* The relevance meter. */}
          <div
            className="h-1 overflow-hidden rounded-full bg-border-soft"
            role="meter"
            aria-valuenow={Math.round(result.score * 100)}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-label="Retrieval relevance"
          >
            <div
              className="h-full rounded-full bg-accent"
              style={{ width: `${Math.round(result.score * 100)}%` }}
            />
          </div>

          {result.heading_path.length > 0 && (
            <div className="flex flex-wrap items-center gap-1 font-sans text-xs text-text-dim">
              {result.heading_path.map((h, i) => (
                <span key={i} className="flex items-center gap-1">
                  {i > 0 && (
                    <ChevronRight className="h-2.5 w-2.5" strokeWidth={2} />
                  )}
                  <span className="text-text-muted">{h}</span>
                </span>
              ))}
            </div>
          )}

          {/* The raw content id — kept for precision, de-emphasized. */}
          <p className="break-all font-mono text-mono-mini text-text-dim">
            {result.content_id}
          </p>
        </div>
      </EntitySection>

      {/* §2 Text */}
      <EntitySection chapter={2} title="Text">
        <pre className="m-0 bg-surface-sunken px-3 py-2.5 font-mono text-[0.8125rem] leading-[1.55] text-text whitespace-pre-wrap max-h-96 overflow-y-auto">
          {result.text}
        </pre>
      </EntitySection>

      {/* §3 Same file — symbols */}
      <EntitySection
        chapter={3}
        title="Same file — symbols"
        meta={symbols === null ? "…" : symbols.length}
      >
        {symbols === null ? (
          <EntitySectionLoading />
        ) : symbols.length === 0 ? (
          <EntitySectionEmpty label="No symbols in this file." />
        ) : (
          <div className="max-h-60 overflow-y-auto">
            {symbols.slice(0, 30).map((s) => (
              <EntityRow
                key={s.id}
                tag={s.kind}
                name={s.name}
                subtitle={s.module_path}
                onClick={() =>
                  openEntity({ kind: "symbol", corpusId, symbol: s })
                }
              />
            ))}
          </div>
        )}
      </EntitySection>

      {/* §4 Recent changes */}
      <EntitySection
        chapter={4}
        title="Recent changes"
        meta={changes === null ? "…" : changes.length}
      >
        {changes === null ? (
          <EntitySectionLoading />
        ) : changes.length === 0 ? (
          <EntitySectionEmpty label="No recent file changes." />
        ) : (
          changes.slice(0, 10).map((e, i) => (
            <EntityRow
              key={i}
              tag={e.kind.toUpperCase()}
              name={e.path.split(/[\\/]/).slice(-2).join("/")}
              meta={`${e.affected_sections.length}§`}
            />
          ))
        )}
      </EntitySection>
    </div>
  );
}
