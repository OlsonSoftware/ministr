/**
 * SymbolView — the EntityPanel's symbol inspector.
 *
 * ONE symbol renderer (aaa-explore-unify-symbol): rather than maintain a second
 * bespoke symbol view, this is now a thin connector over the same
 * `SymbolNeighborhood` the Explore facet uses — so a symbol looks and behaves
 * identically whether you reach it from Explore, a session, or an Ask citation
 * (OOUX "no duplicate renderers"). It fetches the symbol's definition + ref
 * graph + same-file symbols + semantic mentions and renders the neighborhood
 * embedded (the EntityPanel supplies the surrounding chrome), wiring every row
 * to `openEntity` for stacked navigation.
 */
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useEntityPanel, type Entity } from "../../hooks/useEntityPanel";
import type {
  SearchResult,
  SymbolDefinitionDetail,
  SymbolInfo,
  SymbolRef,
} from "../../lib/types";
import { SymbolNeighborhood } from "../code/SymbolNeighborhood";

interface Props {
  entity: Extract<Entity, { kind: "symbol" }>;
}

export function SymbolView({ entity }: Props) {
  const { corpusId, symbol } = entity;
  const { openEntity } = useEntityPanel();

  const [definition, setDefinition] = useState<SymbolDefinitionDetail | null>(null);
  const [refs, setRefs] = useState<SymbolRef[]>([]);
  const [sameFile, setSameFile] = useState<SymbolInfo[]>([]);
  const [mentions, setMentions] = useState<SearchResult[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setDefinition(null);
    setRefs([]);
    setSameFile([]);
    setMentions([]);

    Promise.allSettled([
      invoke<SymbolDefinitionDetail>("symbol_definition", { corpusId, symbolId: symbol.id }),
      invoke<SymbolRef[]>("symbol_references", { corpusId, symbolId: symbol.id }),
      invoke<SymbolInfo[]>("search_symbols", {
        corpusId,
        query: "",
        kind: null,
        filePath: symbol.file_path,
      }),
      invoke<SearchResult[]>("search_corpus", { corpusId, query: symbol.name, topK: 12 }),
    ]).then(([d, r, sf, m]) => {
      if (cancelled) return;
      setDefinition(d.status === "fulfilled" ? d.value : null);
      setRefs(r.status === "fulfilled" ? r.value : []);
      setSameFile(
        sf.status === "fulfilled"
          ? sf.value.filter((s) => s.id !== symbol.id)
          : [],
      );
      setMentions(m.status === "fulfilled" ? m.value : []);
      setLoading(false);
    });

    return () => {
      cancelled = true;
    };
  }, [corpusId, symbol.id, symbol.name, symbol.file_path]);

  // A ref edge carries names + files, not a symbol id — resolve the referencing
  // symbol via search_symbols (the precise+search model), then descend into it.
  async function jumpToRef(r: SymbolRef) {
    try {
      const matches = await invoke<SymbolInfo[]>("search_symbols", {
        corpusId,
        query: r.from_name,
        kind: null,
        filePath: r.from_file,
      });
      const match = matches.find((s) => s.name === r.from_name) ?? matches[0];
      if (match) openEntity({ kind: "symbol", corpusId, symbol: match });
      else openEntity({ kind: "file", corpusId, path: r.from_file });
    } catch {
      openEntity({ kind: "file", corpusId, path: r.from_file });
    }
  }

  return (
    <SymbolNeighborhood
      embedded
      symbolName={symbol.name}
      definition={definition}
      references={refs}
      sameFile={sameFile}
      mentions={mentions}
      loading={loading}
      onGoToDefinition={(filePath) =>
        openEntity({ kind: "file", corpusId, path: filePath })
      }
      onJumpRef={jumpToRef}
      onOpenSymbol={(s) => openEntity({ kind: "symbol", corpusId, symbol: s })}
      onOpenSection={(r) => openEntity({ kind: "section", corpusId, result: r })}
    />
  );
}
