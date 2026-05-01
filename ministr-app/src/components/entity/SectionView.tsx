import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useEntityPanel, type Entity } from "../../hooks/useEntityPanel";
import { EntityRow } from "./EntityRow";
import {
  EntitySection,
  EntitySectionEmpty,
  EntitySectionLoading,
} from "./EntitySection";
import { ChevronRight } from "lucide-react";
import type { CoherenceEvent, SymbolInfo } from "../../lib/types";

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

    const changesP = invoke<CoherenceEvent[]>("recent_coherence_events", {
      limit: 50,
      sinceMs: null,
    });

    Promise.allSettled([symbolsP, changesP]).then(([s, c]) => {
      if (cancelled) return;
      setSymbols(s.status === "fulfilled" ? s.value : []);
      setChanges(
        c.status === "fulfilled"
          ? filePath
            ? c.value.filter((e) =>
                e.path.replace(/\\/g, "/").endsWith(filePath.replace(/\\/g, "/")),
              )
            : c.value
          : [],
      );
    });
    return () => {
      cancelled = true;
    };
  }, [corpusId, filePath, result.content_id]);

  return (
    <div className="flex flex-col gap-4">
      {/* §1 Overview */}
      <EntitySection chapter={1} title="Overview">
        <div className="px-3 py-3 space-y-1.5">
          <div className="flex items-baseline gap-2 flex-wrap">
            <span className="font-mono text-xs font-semibold tabular-nums text-text-muted">
              {(result.score * 100).toFixed(0)}%
            </span>
            <span className="font-mono text-sm text-text break-all">
              {result.content_id}
            </span>
          </div>
          {result.heading_path.length > 0 && (
            <div className="flex items-center gap-1 flex-wrap font-sans text-xs text-text-dim mt-1">
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
