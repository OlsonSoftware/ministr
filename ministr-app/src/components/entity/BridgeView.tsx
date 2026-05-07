import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useEntityPanel, type Entity } from "../../hooks/useEntityPanel";
import { EntityRow } from "./EntityRow";
import {
  EntitySection,
  EntitySectionEmpty,
  EntitySectionLoading,
} from "./EntitySection";
import { MetricTile } from "../ui/metric-tile";
import type { BridgeLink, SymbolInfo } from "../../lib/types";

interface Props {
  entity: Extract<Entity, { kind: "bridge" }>;
}

interface ExcerptState {
  exportSrc: string | null;
  importSrc: string | null;
  exportStart: number | null;
  importStart: number | null;
  loading: boolean;
}

export function BridgeView({ entity }: Props) {
  const { corpusId, link } = entity;
  const { openEntity } = useEntityPanel();

  const [excerpt, setExcerpt] = useState<ExcerptState>({
    exportSrc: null,
    importSrc: null,
    exportStart: null,
    importStart: null,
    loading: true,
  });
  const [otherKind, setOtherKind] = useState<BridgeLink[] | null>(null);
  const [exportSym, setExportSym] = useState<SymbolInfo | null | "loading">("loading");
  const [importSym, setImportSym] = useState<SymbolInfo | null | "loading">("loading");

  useEffect(() => {
    let cancelled = false;
    setExcerpt({
      exportSrc: null,
      importSrc: null,
      exportStart: Math.max(1, link.export_line - 3),
      importStart: Math.max(1, link.import_line - 3),
      loading: true,
    });
    setOtherKind(null);
    setExportSym("loading");
    setImportSym("loading");

    Promise.allSettled([
      invoke<string>("read_source_excerpt", {
        corpusId,
        filePath: link.export_file,
        lineStart: link.export_line,
        lineEnd: link.export_line,
      }),
      invoke<string>("read_source_excerpt", {
        corpusId,
        filePath: link.import_file,
        lineStart: link.import_line,
        lineEnd: link.import_line,
      }),
      invoke<BridgeLink[]>("bridge_query", {
        corpusId,
        query: null,
        kind: link.kind,
        sourceLanguage: null,
        filePath: null,
        limit: 200,
      }),
      invoke<SymbolInfo[]>("search_symbols", {
        corpusId,
        query: link.export_symbol || link.export_binding_key || "",
        kind: null,
        filePath: link.export_file,
      }),
      invoke<SymbolInfo[]>("search_symbols", {
        corpusId,
        query: link.import_symbol || link.import_binding_key || "",
        kind: null,
        filePath: link.import_file,
      }),
    ]).then(([e, i, ok, esym, isym]) => {
      if (cancelled) return;
      setExcerpt({
        exportSrc:
          e.status === "fulfilled" ? e.value : "// (could not read source)",
        importSrc:
          i.status === "fulfilled" ? i.value : "// (could not read source)",
        exportStart: Math.max(1, link.export_line - 3),
        importStart: Math.max(1, link.import_line - 3),
        loading: false,
      });
      setOtherKind(ok.status === "fulfilled" ? ok.value : []);
      setExportSym(
        esym.status === "fulfilled"
          ? esym.value.find(
              (s) => s.name === (link.export_symbol || link.export_binding_key),
            ) ?? esym.value[0] ?? null
          : null,
      );
      setImportSym(
        isym.status === "fulfilled"
          ? isym.value.find(
              (s) => s.name === (link.import_symbol || link.import_binding_key),
            ) ?? isym.value[0] ?? null
          : null,
      );
    });
    return () => {
      cancelled = true;
    };
  }, [corpusId, link]);

  const otherFiltered = otherKind
    ? otherKind.filter(
        (b) =>
          !(
            b.export_file === link.export_file &&
            b.export_line === link.export_line &&
            b.import_file === link.import_file &&
            b.import_line === link.import_line
          ),
      )
    : null;

  return (
    <div className="flex flex-col gap-4">
      {/* §1 Meta */}
      <EntitySection chapter={1} title="Meta">
        <div className="grid grid-cols-3 divide-x divide-border-soft">
          <MetricTile variant="cell" label="Kind" value={link.kind.toUpperCase()} />
          <MetricTile
            variant="cell"
            label="Confidence"
            value={`${(link.confidence * 100).toFixed(0)}%`}
          />
          <MetricTile
            variant="cell"
            label="Langs"
            value={`${link.export_language.toUpperCase()} → ${link.import_language.toUpperCase()}`}
          />
        </div>
      </EntitySection>

      {/* §2 Export + import — paired code panes */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-3">
        <CodePane
          title="Export"
          file={link.export_file}
          line={link.export_line}
          symbol={link.export_symbol}
          binding={link.export_binding_key}
          language={link.export_language}
          source={excerpt.exportSrc}
          startLine={excerpt.exportStart ?? 1}
          loading={excerpt.loading}
        />
        <CodePane
          title="Import"
          file={link.import_file}
          line={link.import_line}
          symbol={link.import_symbol}
          binding={link.import_binding_key}
          language={link.import_language}
          source={excerpt.importSrc}
          startLine={excerpt.importStart ?? 1}
          loading={excerpt.loading}
        />
      </div>

      {/* §3 Symbols */}
      <EntitySection chapter={3} title="Symbols">
        <EntityRow
          tag="export"
          name={link.export_symbol || link.export_binding_key}
          subtitle={`${link.export_language} · ${link.export_file
            .split(/[\\/]/)
            .slice(-2)
            .join("/")}:${link.export_line}`}
          meta={exportSym === "loading" ? "…" : exportSym ? "→" : "unresolved"}
          onClick={
            exportSym && exportSym !== "loading"
              ? () =>
                  openEntity({ kind: "symbol", corpusId, symbol: exportSym })
              : undefined
          }
        />
        <EntityRow
          tag="import"
          name={link.import_symbol || link.import_binding_key}
          subtitle={`${link.import_language} · ${link.import_file
            .split(/[\\/]/)
            .slice(-2)
            .join("/")}:${link.import_line}`}
          meta={importSym === "loading" ? "…" : importSym ? "→" : "unresolved"}
          onClick={
            importSym && importSym !== "loading"
              ? () =>
                  openEntity({ kind: "symbol", corpusId, symbol: importSym })
              : undefined
          }
        />
      </EntitySection>

      {/* §4 Other of kind */}
      <EntitySection
        chapter={4}
        title={`Other ${link.kind.toLowerCase()}`}
        meta={otherFiltered === null ? "…" : otherFiltered.length}
      >
        {otherFiltered === null ? (
          <EntitySectionLoading />
        ) : otherFiltered.length === 0 ? (
          <EntitySectionEmpty label="No other bridges of this kind." />
        ) : (
          <div className="max-h-72 overflow-y-auto">
            {otherFiltered.slice(0, 50).map((b, i) => (
              <EntityRow
                key={`${b.kind}-${i}`}
                tag={b.kind}
                name={`${b.export_symbol || b.export_binding_key} ↔ ${b.import_symbol || b.import_binding_key}`}
                subtitle={`${b.export_language} → ${b.import_language}`}
                meta={`${(b.confidence * 100).toFixed(0)}%`}
                onClick={() => openEntity({ kind: "bridge", corpusId, link: b })}
              />
            ))}
          </div>
        )}
      </EntitySection>
    </div>
  );
}

function CodePane({
  title,
  file,
  line,
  symbol,
  binding,
  language,
  source,
  startLine,
  loading,
}: {
  title: string;
  file: string;
  line: number;
  symbol: string;
  binding: string;
  language: string;
  source: string | null;
  startLine: number;
  loading: boolean;
}) {
  const tail = file.replace(/\\/g, "/").split("/").slice(-2).join("/");
  return (
    <div className="border border-border-soft bg-surface flex flex-col min-h-0">
      <div className="flex items-baseline justify-between border-b border-border-soft bg-surface-overlay px-3 py-2 shrink-0">
        <span className="font-serif text-base font-bold text-text">
          {title}
        </span>
        <span className="font-mono text-mono-mini uppercase tracking-[0.05em] text-text-dim">
          {language}
        </span>
      </div>
      <div className="border-b border-border-soft px-3 py-1.5 shrink-0">
        <span className="font-mono text-sm font-semibold text-text break-words">
          {symbol || binding}
        </span>
        <p className="font-mono text-xs text-text-dim mt-0.5">
          {tail}:{line}
        </p>
      </div>
      {loading ? (
        <p className="px-3 py-2 font-serif text-sm italic text-text-dim">
          Loading<span className="ministr-blink">_</span>
        </p>
      ) : (
        <pre className="m-0 bg-surface-sunken px-3 py-2.5 font-mono text-[0.8125rem] leading-[1.55] text-text whitespace-pre overflow-auto max-h-72">
          {source ?? "// (no source)"}
        </pre>
      )}
      <p className="px-3 py-1 font-serif text-xs italic text-text-dim border-t border-border-soft">
        Starting at line {startLine}.
      </p>
    </div>
  );
}
