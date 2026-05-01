import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useEntityPanel, type Entity } from "../../hooks/useEntityPanel";
import { EntityRow } from "./EntityRow";
import {
  EntitySection,
  EntitySectionEmpty,
  EntitySectionLoading,
} from "./EntitySection";
import { cn } from "../../lib/utils";
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
      invoke<CoherenceEvent[]>("recent_coherence_events", {
        limit: 50,
        sinceMs: null,
      }),
      invoke<FileInfo[]>("list_corpus_files", { corpusId }),
    ]).then(([s, b, c, files]) => {
      if (cancelled) return;
      setSymbols(s.status === "fulfilled" ? s.value : []);
      setBridges(b.status === "fulfilled" ? b.value : []);
      setChanges(
        c.status === "fulfilled"
          ? c.value.filter((e) =>
              e.path.replace(/\\/g, "/").endsWith(path.replace(/\\/g, "/")),
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
      {/* §1 Overview */}
      <EntitySection chapter={1} title="Overview">
        <div className="px-3 py-3 space-y-1.5">
          <p className="font-mono text-base font-bold text-text break-all">
            {path.split(/[\\/]/).pop()}
          </p>
          <p className="font-mono text-xs text-text-dim break-all">
            {path}
          </p>
        </div>
        <div className="grid grid-cols-3 border-t border-border-soft">
          <Stat
            label="Sections"
            value={(meta?.section_count ?? 0).toLocaleString()}
          />
          <Stat label="Symbols" value={(symbols?.length ?? 0).toString()} />
          <Stat label="Bridges" value={(bridges?.length ?? 0).toString()} />
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
                      "inline-flex items-center gap-1.5 border px-2 py-0.5 font-mono text-[0.6875rem] font-semibold uppercase tracking-[0.05em] cursor-pointer transition-none",
                      active
                        ? "border-accent bg-surface-overlay text-text"
                        : "border-border-soft bg-surface text-text-muted hover:border-border hover:text-text",
                    )}
                    style={{ borderRadius: "var(--radius-pill)" }}
                  >
                    {k.toUpperCase()}
                    <span className="opacity-70 tabular-nums">
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
              name={e.path.split(/[\\/]/).slice(-2).join("/")}
              meta={`${e.affected_sections.length}§`}
            />
          ))
        )}
      </EntitySection>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="border-r border-border-soft last:border-r-0 px-3 py-2">
      <p className="font-mono text-[0.6875rem] uppercase tracking-[0.05em] text-text-dim">
        {label}
      </p>
      <p className="font-mono text-base font-semibold tabular-nums text-text mt-0.5">
        {value}
      </p>
    </div>
  );
}
