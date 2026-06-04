/**
 * BridgeMap — the codebase's CROSS-LANGUAGE SEAMS as first-class objects.
 *
 * ministr's signature capability is the bridge graph: where one language calls
 * into another (Tauri commands, PyO3, NAPI, wasm-bindgen, HTTP routes, FFI).
 * Until now that only showed up as "Cross-language" edges inside a symbol's
 * neighborhood or one bridge at a time in the EntityPanel. This is the bespoke
 * MAP: every seam in the project, grouped by mechanism, each bridge an
 * export↔import pair (symbol · file · language on each side) with a confidence
 * cue — filterable by mechanism + language, click-to-inspect (the shared
 * EntityPanel bridge entity) and click-to-navigate (open the file in the code
 * lens). Built fresh from the v4 tokens/atoms — NOT a port of the web graph viz.
 *
 * The pure `BridgeMap` renders from props (Storybook); `BridgeMapConnector`
 * wires the live `bridge_query` invoke + the shared inspector.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowLeftRight,
  Boxes,
  Cable,
  Globe,
  Network,
  Plug,
  Workflow,
} from "lucide-react";

import type { BridgeLink } from "../../lib/types";
import { cn } from "../../lib/utils";
import { useEntityPanel } from "../../hooks/useEntityPanel";
import { LensHeader, LensLoading, LensEmpty, LensRerunButton } from "../ui/lens-frame";

// ── Mechanism → display meta (the seam vocabulary). ────────────────────────
const KIND_META: Record<string, { label: string; icon: typeof Cable }> = {
  tauri_command: { label: "Tauri command", icon: Cable },
  tauri_event: { label: "Tauri event", icon: Cable },
  pyo3_function: { label: "PyO3", icon: Plug },
  pyo3_class: { label: "PyO3 class", icon: Plug },
  napi_export: { label: "NAPI", icon: Plug },
  wasm_bindgen: { label: "wasm-bindgen", icon: Boxes },
  http_route: { label: "HTTP route", icon: Globe },
  ffi: { label: "FFI", icon: Network },
};

function kindMeta(kind: string): { label: string; icon: typeof Cable } {
  return KIND_META[kind] ?? { label: kind.replace(/_/g, " "), icon: Workflow };
}

function fileTail(path: string): string {
  return path.replace(/\\/g, "/").split("/").slice(-2).join("/");
}

function confidenceTone(c: number): string {
  if (c >= 0.8) return "text-success border-success/40";
  if (c >= 0.5) return "text-text-muted border-border";
  return "text-warning border-warning/40";
}

export interface BridgeMapProps {
  links: BridgeLink[];
  loading?: boolean;
  /** Re-map the cross-language seams (a snapshot — re-run after editing). */
  onRefresh?: () => void;
  refreshing?: boolean;
  /** Inspect a bridge in the shared EntityPanel. */
  onInspect: (link: BridgeLink) => void;
  /** Open a file in the code lens. */
  onOpenFile: (path: string) => void;
}

export function BridgeMap({
  links = [],
  loading = false,
  onRefresh,
  refreshing = false,
  onInspect,
  onOpenFile,
}: BridgeMapProps) {
  const [kindFilter, setKindFilter] = useState<string | null>(null);
  const [langFilter, setLangFilter] = useState<string | null>(null);

  // Facets derived from the data — mechanism + language tallies.
  const kinds = useMemo(() => {
    const m = new Map<string, number>();
    for (const b of links) m.set(b.kind, (m.get(b.kind) ?? 0) + 1);
    return [...m.entries()].sort((a, b) => b[1] - a[1]);
  }, [links]);

  const langs = useMemo(() => {
    const m = new Map<string, number>();
    for (const b of links) {
      m.set(b.export_language, (m.get(b.export_language) ?? 0) + 1);
      m.set(b.import_language, (m.get(b.import_language) ?? 0) + 1);
    }
    return [...m.entries()].sort((a, b) => b[1] - a[1]);
  }, [links]);

  const fileCount = useMemo(() => {
    const s = new Set<string>();
    for (const b of links) {
      s.add(b.export_file);
      s.add(b.import_file);
    }
    return s.size;
  }, [links]);

  const filtered = useMemo(
    () =>
      links.filter(
        (b) =>
          (!kindFilter || b.kind === kindFilter) &&
          (!langFilter ||
            b.export_language === langFilter ||
            b.import_language === langFilter),
      ),
    [links, kindFilter, langFilter],
  );

  // Group the filtered links by mechanism, strongest groups first.
  const groups = useMemo(() => {
    const byKind = new Map<string, BridgeLink[]>();
    for (const b of filtered) {
      const arr = byKind.get(b.kind);
      if (arr) arr.push(b);
      else byKind.set(b.kind, [b]);
    }
    return [...byKind.entries()].sort((a, b) => b[1].length - a[1].length);
  }, [filtered]);

  if (loading) {
    return <LensLoading label="Mapping cross-language seams" />;
  }

  if (links.length === 0) {
    return (
      <LensEmpty
        icon={ArrowLeftRight}
        title="No cross-language bridges"
        hint="This project looks single-language — ministr maps Tauri, PyO3, NAPI, wasm-bindgen, HTTP-route and FFI seams the moment a project spans two languages."
        action={
          onRefresh ? (
            <LensRerunButton onRefresh={onRefresh} refreshing={refreshing} />
          ) : undefined
        }
      />
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* ── Glance header + facet filters (shared lens-chrome). ────────── */}
      <LensHeader
        icon={Cable}
        title="Cross-language bridges"
        tone="accent"
        glance={
          <>
            <span className="tabular-nums font-semibold text-text">
              {links.length}
            </span>{" "}
            seams ·{" "}
            <span className="tabular-nums font-semibold text-text">
              {kinds.length}
            </span>{" "}
            mechanisms ·{" "}
            <span className="tabular-nums font-semibold text-text">
              {langs.length}
            </span>{" "}
            languages ·{" "}
            <span className="tabular-nums font-semibold text-text">
              {fileCount}
            </span>{" "}
            files
          </>
        }
        onRefresh={onRefresh}
        refreshing={refreshing}
      >
        {/* Mechanism filter chips. */}
        <div className="flex flex-wrap gap-1.5">
          <FacetChip
            label="All"
            count={links.length}
            active={kindFilter === null}
            onClick={() => setKindFilter(null)}
          />
          {kinds.map(([k, n]) => (
            <FacetChip
              key={k}
              label={kindMeta(k).label}
              icon={kindMeta(k).icon}
              count={n}
              active={kindFilter === k}
              onClick={() => setKindFilter(kindFilter === k ? null : k)}
            />
          ))}
        </div>

        {/* Language filter chips (only when the project spans >1 language). */}
        {langs.length > 1 && (
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
              lang
            </span>
            {langs.map(([l, n]) => (
              <FacetChip
                key={l}
                label={l}
                count={n}
                active={langFilter === l}
                onClick={() => setLangFilter(langFilter === l ? null : l)}
                small
              />
            ))}
          </div>
        )}
      </LensHeader>

      {/* ── The seams, grouped by mechanism. ───────────────────────────── */}
      <div className="min-h-0 flex-1 overflow-y-auto">
        {filtered.length === 0 ? (
          <p className="px-4 py-6 font-mono text-mono-mini text-text-dim">
            No bridges match the current filter.
          </p>
        ) : (
          groups.map(([kind, group]) => {
            const meta = kindMeta(kind);
            const Icon = meta.icon;
            return (
              <section
                key={kind}
                className="border-b border-border-soft last:border-b-0"
              >
                <header className="sticky top-0 z-10 flex items-center gap-2 border-b border-border-soft bg-surface-overlay/95 px-4 py-1.5 backdrop-blur">
                  <Icon className="h-3.5 w-3.5 text-accent" strokeWidth={2.25} />
                  <span className="font-mono text-mono-micro font-bold uppercase tracking-[0.08em] text-text">
                    {meta.label}
                  </span>
                  <span className="ml-auto font-mono text-mono-micro tabular-nums text-text-dim">
                    {group.length}
                  </span>
                </header>
                <div className="divide-y divide-border-soft/60">
                  {group.map((b, i) => (
                    <BridgeRow
                      key={`${b.export_file}:${b.export_line}:${b.import_file}:${b.import_line}:${i}`}
                      link={b}
                      onInspect={() => onInspect(b)}
                      onOpenFile={onOpenFile}
                    />
                  ))}
                </div>
              </section>
            );
          })
        )}
      </div>
    </div>
  );
}

// ── One seam: export ↔ import, each side navigable; the row inspects. ───────
function BridgeRow({
  link,
  onInspect,
  onOpenFile,
}: {
  link: BridgeLink;
  onInspect: () => void;
  onOpenFile: (path: string) => void;
}) {
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onInspect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onInspect();
        }
      }}
      title="Inspect this bridge"
      className="group grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-2 px-4 py-2 cursor-pointer hover:bg-surface-overlay transition-colors duration-150 ease-out"
    >
      <Endpoint
        language={link.export_language}
        symbol={link.export_symbol || link.export_binding_key}
        file={link.export_file}
        line={link.export_line}
        onOpenFile={onOpenFile}
        align="start"
      />

      <div className="flex flex-col items-center gap-0.5 px-1">
        <ArrowLeftRight
          className="h-3.5 w-3.5 text-text-dim group-hover:text-accent transition-colors duration-150"
          strokeWidth={2.25}
        />
        <span
          className={cn(
            "rounded-full border px-1 font-mono text-mono-micro tabular-nums leading-none",
            confidenceTone(link.confidence),
          )}
          title="Detection confidence"
        >
          {Math.round(link.confidence * 100)}
        </span>
      </div>

      <Endpoint
        language={link.import_language}
        symbol={link.import_symbol || link.import_binding_key}
        file={link.import_file}
        line={link.import_line}
        onOpenFile={onOpenFile}
        align="end"
      />
    </div>
  );
}

function Endpoint({
  language,
  symbol,
  file,
  line,
  onOpenFile,
  align,
}: {
  language: string;
  symbol: string;
  file: string;
  line: number;
  onOpenFile: (path: string) => void;
  align: "start" | "end";
}) {
  return (
    <div
      className={cn(
        "flex min-w-0 flex-col gap-0.5",
        align === "end" ? "items-end text-right" : "items-start",
      )}
    >
      <div className="flex max-w-full items-center gap-1.5">
        <span className="shrink-0 rounded border border-border-soft bg-surface px-1 font-mono text-mono-micro font-semibold uppercase tracking-[0.06em] text-text-dim">
          {language}
        </span>
        <span className="truncate font-mono text-xs font-semibold text-text">
          {symbol}
        </span>
      </div>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onOpenFile(file);
        }}
        title={`Open ${file}`}
        className="max-w-full truncate font-mono text-mono-micro text-text-dim hover:text-accent cursor-pointer transition-colors duration-150"
      >
        {fileTail(file)}:{line}
      </button>
    </div>
  );
}

function FacetChip({
  label,
  count,
  active,
  onClick,
  icon: Icon,
  small = false,
}: {
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
  icon?: typeof Cable;
  small?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "inline-flex items-center gap-1 rounded-md border font-mono uppercase tracking-[0.06em] cursor-pointer transition-colors duration-150 ease-out",
        small ? "px-1.5 py-0.5 text-mono-micro" : "px-2 py-0.5 text-mono-mini",
        active
          ? "border-accent bg-surface-overlay text-text"
          : "border-border-soft bg-surface text-text-muted hover:border-border hover:text-text",
      )}
    >
      {Icon && <Icon className="h-3 w-3" strokeWidth={2.25} />}
      <span className={small ? "" : "font-semibold"}>{label}</span>
      <span className="tabular-nums opacity-70">{count}</span>
    </button>
  );
}

// ───────────────────────────────────────────────────────────────────────────
// Connector — fetches every bridge in the corpus and wires the shared inspector.

export function BridgeMapConnector({
  corpusId,
  onOpenFile,
}: {
  corpusId: string;
  onOpenFile: (path: string) => void;
}) {
  const { openEntity } = useEntityPanel();
  const [links, setLinks] = useState<BridgeLink[] | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const reqRef = useRef(0);

  const load = useCallback(() => {
    const id = ++reqRef.current;
    setRefreshing(true);
    invoke<BridgeLink[]>("bridge_query", {
      corpusId,
      query: null,
      kind: null,
      sourceLanguage: null,
      filePath: null,
      limit: 500,
    })
      .then((r) => {
        if (reqRef.current === id) setLinks(r);
      })
      .catch(() => {
        if (reqRef.current === id) setLinks([]);
      })
      .finally(() => {
        if (reqRef.current === id) setRefreshing(false);
      });
  }, [corpusId]);

  useEffect(() => {
    setLinks(null);
    load();
  }, [load]);

  return (
    <BridgeMap
      links={links ?? []}
      loading={links === null}
      onRefresh={load}
      refreshing={refreshing}
      onInspect={(link) => openEntity({ kind: "bridge", corpusId, link })}
      onOpenFile={onOpenFile}
    />
  );
}
