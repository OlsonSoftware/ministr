import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FileCode } from "lucide-react";
import { useEntityPanel, type Entity } from "../../hooks/useEntityPanel";
import { EntityRow } from "./EntityRow";
import {
  EntitySection,
  EntitySectionEmpty,
  EntitySectionLoading,
} from "./EntitySection";
import { cn } from "../../lib/utils";
import { MetricTile } from "../ui/metric-tile";
import type {
  BridgeLink,
  CoherenceEvent,
  FileInfo,
  SymbolInfo,
} from "../../lib/types";

interface Props {
  entity: Extract<Entity, { kind: "file" }>;
}

const KIND_FILTERS = ["fn", "struct", "trait", "enum", "impl", "type"] as const;

export function FileView({ entity }: Props) {
  const { corpusId, path } = entity;
  const { openEntity } = useEntityPanel();

  const [symbols, setSymbols] = useState<SymbolInfo[] | null>(null);
  const [bridges, setBridges] = useState<BridgeLink[] | null>(null);
  const [changes, setChanges] = useState<CoherenceEvent[] | null>(null);
  const [meta, setMeta] = useState<FileInfo | null>(null);
  const [activeKinds, setActiveKinds] = useState<Set<string>>(new Set());

  useEffect(() => {
    let cancelled = false;
    setSymbols(null);
    setBridges(null);
    setChanges(null);
    setMeta(null);

    Promise.allSettled([
      invoke<SymbolInfo[]>("search_symbols", {
        corpusId,
        query: "",
        kind: null,
        filePath: path,
      }),
      invoke<BridgeLink[]>("bridge_query", {
        corpusId,
        query: null,
        kind: null,
        sourceLanguage: null,
        filePath: path,
        limit: 200,
      }),
      // Bump the candidate window from 50 to 500: the daemon doesn't
      // yet support server-side filter args for recent_coherence_events,
      // so we filter client-side and need enough headroom that a busy
      // multi-file corpus doesn't push *this* file's older changes off
      // the back. Display still slices to 10 rows below.
      invoke<CoherenceEvent[]>("recent_coherence_events", {
        limit: 500,
        sinceMs: null,
      }),
      invoke<FileInfo[]>("list_corpus_files", { corpusId }),
    ]).then(([s, b, c, files]) => {
      if (cancelled) return;
      setSymbols(s.status === "fulfilled" ? s.value : []);
      setBridges(b.status === "fulfilled" ? b.value : []);
      // Compare normalized full paths, not endsWith(): two corpus roots
      // ending in the same relative path (e.g. apps/a/src/lib.rs vs
      // apps/b/src/lib.rs) would otherwise cross-attribute each other's
      // events. `path` is already the exact file the panel is scoped to.
      setChanges(
        c.status === "fulfilled"
          ? c.value.filter(
              (e) => e.path.replace(/\\/g, "/") === path.replace(/\\/g, "/"),
            )
          : [],
      );
      setMeta(
        files.status === "fulfilled"
          ? files.value.find((f) => f.path === path) ?? null
          : null,
      );
    });
    return () => {
      cancelled = true;
    };
  }, [corpusId, path]);

  const visibleSymbols = useMemo(() => {
    if (!symbols) return null;
    if (activeKinds.size === 0) return symbols;
    return symbols.filter((s) => activeKinds.has(s.kind));
  }, [symbols, activeKinds]);

  const symbolsByKind = useMemo(() => {
    const m = new Map<string, number>();
    if (!symbols) return m;
    for (const s of symbols) m.set(s.kind, (m.get(s.kind) ?? 0) + 1);
    return m;
  }, [symbols]);

  function toggleKind(k: string) {
    setActiveKinds((prev) => {
      const next = new Set(prev);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });
  }

  return (
    <div className="flex flex-col gap-4">
      {/* §1 Overview — a command-deck source-identity header (mirrors
          SectionView): a file medallion + the filename + the full path, over
          the file's vital readout. */}
      <EntitySection chapter={1} title="Overview">
        <div className="flex items-start gap-3 px-3 py-3">
          {/* Quiet accent medallion — a file isn't "live", so no glow. */}
          <span
            aria-hidden
            className="grid h-11 w-11 shrink-0 place-items-center rounded-xl border border-accent/40 bg-surface-overlay text-accent"
          >
            <FileCode className="h-[18px] w-[18px]" strokeWidth={2} />
          </span>
          <div className="min-w-0 flex-1">
            <p className="break-all font-mono text-[15px] font-semibold leading-tight text-text">
              {path.split(/[\\/]/).pop()}
            </p>
            <p className="mt-0.5 break-all font-mono text-mono-mini text-text-dim">
              {path}
            </p>
          </div>
        </div>
        <div className="grid grid-cols-3 border-t border-border-soft divide-x divide-border-soft">
          <MetricTile
            variant="cell"
            label="Sections"
            value={(meta?.section_count ?? 0).toLocaleString()}
          />
          <MetricTile
            variant="cell"
            label="Symbols"
            value={(symbols?.length ?? 0).toString()}
          />
          <MetricTile
            variant="cell"
            label="Bridges"
            value={(bridges?.length ?? 0).toString()}
          />
        </div>
      </EntitySection>

      {/* §2 Symbols */}
      <EntitySection
        chapter={2}
        title="Symbols"
        meta={
          visibleSymbols === null
            ? "…"
            : `${visibleSymbols.length}/${symbols?.length ?? 0}`
        }
      >
        {symbols === null ? (
          <EntitySectionLoading />
        ) : symbols.length === 0 ? (
          <EntitySectionEmpty label="No symbols indexed in this file." />
        ) : (
          <>
            <div className="flex flex-wrap gap-1.5 px-3 py-2 border-b border-border-soft">
              {KIND_FILTERS.filter((k) => symbolsByKind.has(k)).map((k) => {
                const active = activeKinds.has(k);
                return (
                  <button
                    key={k}
                    onClick={() => toggleKind(k)}
                    className={cn(
                      "inline-flex items-center gap-1.5 border px-2 py-0.5 font-mono text-mono-mini font-semibold uppercase tracking-[0.08em] cursor-pointer transition-colors duration-150 ease-out rounded-md",
                      active
                        ? "border-accent bg-surface-overlay text-text"
                        : "border-border-soft bg-surface text-text-muted hover:border-border hover:text-text",
                    )}
                  >
                    {k.toUpperCase()}
                    <span className="tabular-nums">
                      {symbolsByKind.get(k)}
                    </span>
                  </button>
                );
              })}
            </div>
            <div className="max-h-80 overflow-y-auto">
              {visibleSymbols!.slice(0, 80).map((s) => (
                <EntityRow
                  key={s.id}
                  tag={s.kind}
                  name={s.name}
                  subtitle={s.module_path}
                  meta={s.visibility}
                  onClick={() =>
                    openEntity({ kind: "symbol", corpusId, symbol: s })
                  }
                />
              ))}
            </div>
          </>
        )}
      </EntitySection>

      {/* §3 Bridges involving */}
      <EntitySection
        chapter={3}
        title="Bridges involving"
        meta={bridges === null ? "…" : bridges.length}
      >
        {bridges === null ? (
          <EntitySectionLoading />
        ) : bridges.length === 0 ? (
          <EntitySectionEmpty label="No bridges touch this file." />
        ) : (
          bridges.slice(0, 30).map((b, i) => (
            <EntityRow
              key={`${b.kind}-${i}`}
              tag={b.kind}
              name={`${b.export_symbol || b.export_binding_key} ↔ ${b.import_symbol || b.import_binding_key}`}
              subtitle={`${b.export_language} → ${b.import_language}`}
              meta={`${(b.confidence * 100).toFixed(0)}%`}
              onClick={() => openEntity({ kind: "bridge", corpusId, link: b })}
            />
          ))
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
          <EntitySectionEmpty label="No recent changes." />
        ) : (
          changes.slice(0, 10).map((e, i) => (
            <EntityRow
              key={i}
              tag={e.kind.toUpperCase()}
              name={new Date(e.timestamp_ms).toLocaleString()}
              subtitle={`${e.affected_sections.length} affected section${
                e.affected_sections.length === 1 ? "" : "s"
              }`}
              meta={`${e.duration_ms}ms`}
            />
          ))
        )}
      </EntitySection>
    </div>
  );
}

