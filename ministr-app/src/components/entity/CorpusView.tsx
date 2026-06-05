import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Boxes } from "@/components/ui/icons";
import { useEntityPanel, type Entity } from "../../hooks/useEntityPanel";
import { useSessions } from "../../hooks/useSessions";
import { EntityRow } from "./EntityRow";
import {
  EntitySection,
  EntitySectionEmpty,
  EntitySectionLoading,
} from "./EntitySection";
import { CorpusConfigEditor } from "./CorpusConfigEditor";
import { corpusLabel } from "../../lib/corpus";
import { toneTextClass } from "../../lib/status";
import { statusLabel, utilizationTone } from "../../lib/sessions";
import { formatTokens } from "../../lib/format";
import { MetricTile } from "../ui/metric-tile";
import type { CoherenceEvent, FileInfo } from "../../lib/types";

interface Props {
  entity: Extract<Entity, { kind: "corpus" }>;
}

export function CorpusView({ entity }: Props) {
  const { corpus } = entity;
  const { openEntity } = useEntityPanel();

  // Sessions come from the one shared store (single poll app-wide).
  const { sessions: allSessions, loaded: sessionsLoaded } = useSessions();
  const sessions = useMemo(
    () => allSessions.filter((s) => s.corpus_id === corpus.id),
    [allSessions, corpus.id],
  );

  const [files, setFiles] = useState<FileInfo[] | null>(null);
  const [changes, setChanges] = useState<CoherenceEvent[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    setFiles(null);
    setChanges(null);

    Promise.allSettled([
      invoke<FileInfo[]>("list_corpus_files", { corpusId: corpus.id }),
      // Bump the candidate window so a corpus with slightly older
      // changes still appears active after sibling corpora produce
      // 50+ newer events. Display still slices to 12 rows below.
      invoke<CoherenceEvent[]>("recent_coherence_events", {
        limit: 500,
        sinceMs: null,
      }),
    ]).then(([f, c]) => {
      if (cancelled) return;
      setFiles(f.status === "fulfilled" ? f.value : []);
      setChanges(
        c.status === "fulfilled"
          ? c.value.filter((e) => e.corpus_id === corpus.id)
          : [],
      );
    });
    return () => {
      cancelled = true;
    };
  }, [corpus.id]);

  const topFiles = files
    ? [...files].sort((a, b) => b.section_count - a.section_count).slice(0, 20)
    : null;

  return (
    <div className="flex flex-col gap-4">
      {/* §1 Overview — a command-deck identity header (the corpus = the
          project; uses the SAME Boxes glyph as its ScopeHeader banner) over
          the paths, model, and per-corpus config editor. */}
      <EntitySection chapter={1} title="Overview">
        <div className="space-y-2.5 px-3 py-3">
          <div className="flex items-start gap-3">
            {/* Quiet accent medallion — no glow (the inspector is static). */}
            <span
              aria-hidden
              className="grid h-11 w-11 shrink-0 place-items-center rounded-xl border border-accent/40 bg-surface-overlay text-accent"
            >
              <Boxes className="h-[18px] w-[18px]" strokeWidth={2} />
            </span>
            <div className="min-w-0 flex-1">
              <p className="break-all font-mono text-[15px] font-semibold leading-tight text-text">
                {corpusLabel(corpus)}
              </p>
              {corpus.paths.map((p) => (
                <p
                  key={p}
                  className="mt-0.5 break-all font-mono text-mono-mini text-text-dim"
                >
                  {p}
                </p>
              ))}
              {corpus.model ? (
                <p className="mt-1 font-mono text-mono-mini text-text-dim">
                  model <span className="text-text">{corpus.model}</span>
                </p>
              ) : null}
            </div>
          </div>
          <CorpusConfigEditor corpus={corpus} />
        </div>
      </EntitySection>

      {/* §2 Stats */}
      <EntitySection chapter={2} title="Stats">
        <div className="grid grid-cols-2 divide-x divide-y divide-border-soft">
          <MetricTile
            variant="cell"
            label="Files"
            value={corpus.files_indexed.toLocaleString()}
          />
          <MetricTile
            variant="cell"
            label="Sections"
            value={corpus.sections_count.toLocaleString()}
          />
          <MetricTile
            variant="cell"
            label="Symbols"
            value={(corpus.symbols_count ?? 0).toLocaleString()}
          />
          <MetricTile
            variant="cell"
            label="Vectors"
            value={corpus.embeddings_count.toLocaleString()}
          />
        </div>
      </EntitySection>

      {/* §3 Active sessions */}
      <EntitySection
        chapter={3}
        title="Active sessions"
        meta={!sessionsLoaded ? "…" : sessions.length}
      >
        {!sessionsLoaded ? (
          <EntitySectionLoading />
        ) : sessions.length === 0 ? (
          <EntitySectionEmpty label="No active sessions." />
        ) : (
          sessions.map((s) => {
            const tone = utilizationTone(s.utilization);
            return (
              <EntityRow
                key={s.session_id}
                tag={statusLabel(tone)}
                name={s.session_id.slice(0, 12)}
                subtitle={`turn ${s.current_turn} · ${formatTokens(s.tokens_used)} tokens`}
                meta={`${(s.utilization * 100).toFixed(0)}%`}
                onClick={() =>
                  openEntity({
                    kind: "session",
                    corpusId: corpus.id,
                    sessionId: s.session_id,
                    seed: s,
                  })
                }
                className={toneTextClass(tone)}
              />
            );
          })
        )}
      </EntitySection>

      {/* §4 Hot files */}
      <EntitySection
        chapter={4}
        title="Hot files"
        meta={topFiles === null ? "…" : topFiles.length}
      >
        {topFiles === null ? (
          <EntitySectionLoading />
        ) : topFiles.length === 0 ? (
          <EntitySectionEmpty label="No files indexed yet." />
        ) : (
          <div className="max-h-72 overflow-y-auto">
            {topFiles.map((f) => (
              <EntityRow
                key={f.path}
                name={f.path.split(/[\\/]/).pop() ?? f.path}
                subtitle={f.path}
                meta={`${f.section_count}§`}
                onClick={() =>
                  openEntity({ kind: "file", corpusId: corpus.id, path: f.path })
                }
              />
            ))}
          </div>
        )}
      </EntitySection>

      {/* §5 Recent changes */}
      <EntitySection
        chapter={5}
        title="Recent changes"
        meta={changes === null ? "…" : changes.length}
      >
        {changes === null ? (
          <EntitySectionLoading />
        ) : changes.length === 0 ? (
          <EntitySectionEmpty label="No recent changes." />
        ) : (
          changes.slice(0, 12).map((e, i) => (
            <EntityRow
              key={i}
              tag={e.kind.toUpperCase()}
              name={e.path.split(/[\\/]/).slice(-2).join("/")}
              meta={`${e.affected_sections.length}§`}
              onClick={() =>
                openEntity({
                  kind: "file",
                  corpusId: corpus.id,
                  path: e.path,
                })
              }
            />
          ))
        )}
      </EntitySection>
    </div>
  );
}

