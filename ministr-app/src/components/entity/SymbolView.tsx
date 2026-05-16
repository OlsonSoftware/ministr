import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useEntityPanel, type Entity } from "../../hooks/useEntityPanel";
import { cn } from "../../lib/utils";
import { EntityRow } from "./EntityRow";
import {
  EntitySection,
  EntitySectionEmpty,
  EntitySectionLoading,
} from "./EntitySection";
import type {
  BridgeLink,
  SearchResult,
  SymbolDefinitionDetail,
  SymbolInfo,
  SymbolRef,
} from "../../lib/types";

interface Props {
  entity: Extract<Entity, { kind: "symbol" }>;
}

const REF_KIND_LABELS: Record<string, string> = {
  calls: "CALLS",
  imports: "IMPORTS",
  implements: "IMPL",
  uses: "USES",
};

export function SymbolView({ entity }: Props) {
  const { corpusId, symbol } = entity;
  const { openEntity } = useEntityPanel();

  const [definition, setDefinition] = useState<SymbolDefinitionDetail | null>(
    null,
  );
  const [refs, setRefs] = useState<SymbolRef[] | null>(null);
  const [bridges, setBridges] = useState<BridgeLink[] | null>(null);
  const [sameFile, setSameFile] = useState<SymbolInfo[] | null>(null);
  const [mentions, setMentions] = useState<SearchResult[] | null>(null);
  const [activeRefKinds, setActiveRefKinds] = useState<Set<string>>(new Set());

  useEffect(() => {
    let cancelled = false;
    setDefinition(null);
    setRefs(null);
    setBridges(null);
    setSameFile(null);
    setMentions(null);

    Promise.allSettled([
      invoke<SymbolDefinitionDetail>("symbol_definition", {
        corpusId,
        symbolId: symbol.id,
      }),
      invoke<SymbolRef[]>("symbol_references", {
        corpusId,
        symbolId: symbol.id,
      }),
      invoke<BridgeLink[]>("bridge_query", {
        corpusId,
        query: symbol.name,
        kind: null,
        sourceLanguage: null,
        filePath: null,
        limit: 200,
      }),
      invoke<SymbolInfo[]>("search_symbols", {
        corpusId,
        query: "",
        kind: null,
        filePath: symbol.file_path,
      }),
      invoke<SearchResult[]>("search_corpus", {
        corpusId,
        query: symbol.name,
        topK: 12,
      }),
    ]).then(([d, r, b, sf, m]) => {
      if (cancelled) return;
      setDefinition(d.status === "fulfilled" ? d.value : null);
      setRefs(r.status === "fulfilled" ? r.value : []);
      setBridges(b.status === "fulfilled" ? b.value : []);
      setSameFile(sf.status === "fulfilled" ? sf.value : []);
      setMentions(m.status === "fulfilled" ? m.value : []);
    });
    return () => {
      cancelled = true;
    };
  }, [corpusId, symbol.id, symbol.name, symbol.file_path]);

  // Derived: bridges-out (this symbol is the export side), bridges-in (import).
  const { bridgesOut, bridgesIn } = useMemo(() => {
    if (!bridges) return { bridgesOut: null, bridgesIn: null };
    const out: BridgeLink[] = [];
    const inn: BridgeLink[] = [];
    for (const b of bridges) {
      if (
        b.export_symbol === symbol.name ||
        b.export_binding_key === symbol.name
      ) {
        out.push(b);
      }
      if (
        b.import_symbol === symbol.name ||
        b.import_binding_key === symbol.name
      ) {
        inn.push(b);
      }
    }
    return { bridgesOut: out, bridgesIn: inn };
  }, [bridges, symbol.name]);

  const refsByKind = useMemo(() => {
    if (!refs) return new Map<string, number>();
    const m = new Map<string, number>();
    for (const r of refs) m.set(r.ref_kind, (m.get(r.ref_kind) ?? 0) + 1);
    return m;
  }, [refs]);

  const visibleRefs = useMemo(() => {
    if (!refs) return [];
    if (activeRefKinds.size === 0) return refs;
    return refs.filter((r) => activeRefKinds.has(r.ref_kind));
  }, [refs, activeRefKinds]);

  // Same-file symbols excluding the currently-open one.
  const sameFileFiltered = useMemo(() => {
    if (!sameFile) return null;
    return sameFile.filter((s) => s.id !== symbol.id);
  }, [sameFile, symbol.id]);

  function toggleRefKind(k: string) {
    setActiveRefKinds((prev) => {
      const next = new Set(prev);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });
  }

  // To open a Symbol from a SymbolRef row, we need a SymbolInfo. Best-effort
  // resolve via search_symbols by name; fall back to a synthetic stub.
  async function jumpToFromSymbol(r: SymbolRef) {
    try {
      const matches = await invoke<SymbolInfo[]>("search_symbols", {
        corpusId,
        query: r.from_name,
        kind: null,
        filePath: r.from_file,
      });
      const match =
        matches.find((s) => s.name === r.from_name) ?? matches[0];
      if (match) {
        openEntity({ kind: "symbol", corpusId, symbol: match });
      }
    } catch {
      /* ignore */
    }
  }

  return (
    <div className="flex flex-col gap-4">
      {/* §1 Overview */}
      <EntitySection chapter={1} title="Overview">
        <div className="px-3 py-3 space-y-1.5">
          <div className="flex items-baseline gap-2 flex-wrap">
            <span className="font-mono text-mono-mini font-semibold uppercase tracking-[0.08em] text-text-dim">
              {symbol.kind}
            </span>
            <span className="font-mono text-base font-bold text-text">
              {symbol.name}
            </span>
            {symbol.visibility && (
              <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
                {symbol.visibility}
              </span>
            )}
          </div>
          <div className="font-mono text-xs text-text-dim break-all">
            {symbol.file_path}
            {definition && `:${definition.line_start}-${definition.line_end}`}
          </div>
          {symbol.module_path && (
            <div className="font-mono text-xs text-text-dim">
              {symbol.module_path}
            </div>
          )}
        </div>
      </EntitySection>

      {/* §2 Signature */}
      {symbol.signature && (
        <EntitySection chapter={2} title="Signature">
          <pre className="m-0 px-3 py-2.5 font-mono text-[0.8125rem] leading-[1.5] text-text whitespace-pre-wrap break-words bg-surface-sunken">
            {symbol.signature}
          </pre>
        </EntitySection>
      )}

      {/* §3 Docs */}
      {definition?.doc_comment && (
        <EntitySection chapter={3} title="Docs">
          <pre className="m-0 border-l-2 border-border-soft bg-surface-overlay px-3 py-2.5 font-sans text-sm italic text-text-muted whitespace-pre-wrap leading-relaxed">
            {definition.doc_comment}
          </pre>
        </EntitySection>
      )}

      {/* §4 Source — line-numbered gutter, larger code, tighter leading. */}
      <EntitySection chapter={4} title="Source">
        {definition === null ? (
          <EntitySectionLoading />
        ) : !definition.source_context ? (
          <EntitySectionEmpty label="No source available." />
        ) : (
          <SourceBlock
            source={definition.source_context}
            startLine={definition.line_start}
          />
        )}
      </EntitySection>

      {/* §5 References */}
      <EntitySection
        chapter={5}
        title="References"
        meta={refs === null ? "…" : `${visibleRefs.length}/${refs.length}`}
      >
        {refs === null ? (
          <EntitySectionLoading />
        ) : refs.length === 0 ? (
          <EntitySectionEmpty label="No references." />
        ) : (
          <>
            <div className="flex flex-wrap gap-1.5 px-3 py-2 border-b border-border-soft">
              {Array.from(refsByKind.entries()).map(([k, c]) => {
                const active = activeRefKinds.has(k);
                return (
                  <button
                    key={k}
                    onClick={() => toggleRefKind(k)}
                    className={cn(
                      "inline-flex items-center gap-1.5 border px-2 py-0.5 font-mono text-mono-mini font-semibold uppercase tracking-[0.08em] cursor-pointer transition-colors duration-150 ease-out",
                      active
                        ? "border-accent bg-surface-overlay text-text"
                        : "border-border-soft bg-surface text-text-muted hover:border-border hover:text-text",
                    "rounded-md")}
                  >
                    {REF_KIND_LABELS[k] ?? k.toUpperCase()}
                    <span className="opacity-70 tabular-nums">{c}</span>
                  </button>
                );
              })}
            </div>
            <div className="max-h-72 overflow-y-auto">
              {visibleRefs.map((r, i) => (
                <EntityRow
                  key={i}
                  tag={REF_KIND_LABELS[r.ref_kind] ?? r.ref_kind.toUpperCase()}
                  name={r.from_name}
                  subtitle={r.from_file}
                  meta={`→ ${r.to_name}`}
                  onClick={() => jumpToFromSymbol(r)}
                />
              ))}
            </div>
          </>
        )}
      </EntitySection>

      {/* §6 Bridges — export */}
      <EntitySection
        chapter={6}
        title="Bridges — export"
        meta={bridgesOut === null ? "…" : bridgesOut.length}
      >
        {bridgesOut === null ? (
          <EntitySectionLoading />
        ) : bridgesOut.length === 0 ? (
          <EntitySectionEmpty label="No export bridges." />
        ) : (
          bridgesOut.map((b, i) => (
            <EntityRow
              key={`${b.kind}-${i}`}
              tag={b.kind}
              name={b.import_symbol || b.import_binding_key}
              subtitle={`${b.export_language} → ${b.import_language}`}
              meta={`${(b.confidence * 100).toFixed(0)}%`}
              onClick={() =>
                openEntity({ kind: "bridge", corpusId, link: b })
              }
            />
          ))
        )}
      </EntitySection>

      {/* §7 Bridges — import */}
      <EntitySection
        chapter={7}
        title="Bridges — import"
        meta={bridgesIn === null ? "…" : bridgesIn.length}
      >
        {bridgesIn === null ? (
          <EntitySectionLoading />
        ) : bridgesIn.length === 0 ? (
          <EntitySectionEmpty label="No import bridges." />
        ) : (
          bridgesIn.map((b, i) => (
            <EntityRow
              key={`${b.kind}-${i}`}
              tag={b.kind}
              name={b.export_symbol || b.export_binding_key}
              subtitle={`${b.export_language} → ${b.import_language}`}
              meta={`${(b.confidence * 100).toFixed(0)}%`}
              onClick={() =>
                openEntity({ kind: "bridge", corpusId, link: b })
              }
            />
          ))
        )}
      </EntitySection>

      {/* §8 Same file — symbols */}
      <EntitySection
        chapter={8}
        title="Same file — symbols"
        meta={sameFileFiltered === null ? "…" : sameFileFiltered.length}
      >
        {sameFileFiltered === null ? (
          <EntitySectionLoading />
        ) : sameFileFiltered.length === 0 ? (
          <EntitySectionEmpty label="No other symbols in this file." />
        ) : (
          <div className="max-h-60 overflow-y-auto">
            {sameFileFiltered.slice(0, 30).map((s) => (
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

      {/* §9 Mentions — appended after the eight reserved chapters. */}
      <EntitySection
        chapter={9}
        title="Mentions"
        meta={mentions === null ? "…" : mentions.length}
      >
        {mentions === null ? (
          <EntitySectionLoading />
        ) : mentions.length === 0 ? (
          <EntitySectionEmpty label="No mentions in corpus." />
        ) : (
          <div className="max-h-60 overflow-y-auto">
            {mentions.map((r, i) => {
              const id = r.content_id.replace(/\\/g, "/");
              const tail = id.split("/").slice(-2).join("/");
              return (
                <EntityRow
                  key={i}
                  tag={`${(r.score * 100).toFixed(0)}%`}
                  name={tail}
                  subtitle={r.heading_path.join(" / ") || undefined}
                  onClick={() =>
                    openEntity({ kind: "section", corpusId, result: r })
                  }
                />
              );
            })}
          </div>
        )}
      </EntitySection>
    </div>
  );
}

/** Field-manual source block: left-gutter line numbers + 13px mono. */
function SourceBlock({
  source,
  startLine,
}: {
  source: string;
  startLine: number;
}) {
  const lines = source.replace(/\n$/, "").split("\n");
  const lastLineNo = startLine + lines.length - 1;
  const gutterWidth = String(lastLineNo).length;
  return (
    <div className="bg-surface-sunken max-h-72 overflow-y-auto">
      <pre className="m-0 font-mono text-[0.8125rem] leading-[1.55] text-text whitespace-pre overflow-x-auto">
        {lines.map((line, i) => (
          <div key={i} className="flex">
            <span
              aria-hidden="true"
              className="select-none shrink-0 px-3 py-0 text-text-dim text-right tabular-nums"
              style={{
                minWidth: `${gutterWidth + 2}ch`,
                borderRight: "1px solid var(--color-border-soft)",
              }}
            >
              {startLine + i}
            </span>
            <span className="px-3 flex-1">{line || " "}</span>
          </div>
        ))}
      </pre>
    </div>
  );
}
