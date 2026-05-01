import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useEntityPanel, type Entity } from "../../hooks/useEntityPanel";
import { EntityRow } from "./EntityRow";
import {
  EntitySection,
  EntitySectionEmpty,
  EntitySectionLoading,
} from "./EntitySection";
import { corpusLabel } from "../../lib/corpus";
import { pressureTone, toneTextClass } from "../../lib/status";
import { cn } from "../../lib/utils";
import { formatTokens } from "../../lib/format";
import type {
  CoherenceEvent,
  FileInfo,
  SessionDetail,
} from "../../lib/types";

interface Props {
  entity: Extract<Entity, { kind: "corpus" }>;
}

export function CorpusView({ entity }: Props) {
  const { corpus } = entity;
  const { openEntity } = useEntityPanel();

  const [files, setFiles] = useState<FileInfo[] | null>(null);
  const [sessions, setSessions] = useState<SessionDetail[] | null>(null);
  const [changes, setChanges] = useState<CoherenceEvent[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    setFiles(null);
    setSessions(null);
    setChanges(null);

    Promise.allSettled([
      invoke<FileInfo[]>("list_corpus_files", { corpusId: corpus.id }),
      invoke<SessionDetail[]>("list_sessions"),
      invoke<CoherenceEvent[]>("recent_coherence_events", {
        limit: 50,
        sinceMs: null,
      }),
    ]).then(([f, s, c]) => {
      if (cancelled) return;
      setFiles(f.status === "fulfilled" ? f.value : []);
      setSessions(
        s.status === "fulfilled"
          ? s.value.filter((x) => x.corpus_id === corpus.id)
          : [],
      );
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
      {/* §1 Overview */}
      <EntitySection chapter={1} title="Overview">
        <div className="px-3 py-3 space-y-1.5">
          <p className="font-mono text-base font-bold text-text break-all">
            {corpusLabel(corpus)}
          </p>
          {corpus.paths.map((p) => (
            <p
              key={p}
              className="font-mono text-xs text-text-dim break-all"
            >
              {p}
            </p>
          ))}
        </div>
      </EntitySection>

      {/* §2 Stats */}
      <EntitySection chapter={2} title="Stats">
        <div className="grid grid-cols-2">
          <Stat
            label="Files"
            value={corpus.files_indexed.toLocaleString()}
          />
          <Stat
            label="Sections"
            value={corpus.sections_count.toLocaleString()}
          />
          <Stat
            label="Symbols"
            value={(corpus.symbols_count ?? 0).toLocaleString()}
          />
          <Stat
            label="Vectors"
            value={corpus.embeddings_count.toLocaleString()}
          />
        </div>
      </EntitySection>

      {/* §3 Active sessions */}
      <EntitySection
        chapter={3}
        title="Active sessions"
        meta={sessions === null ? "…" : sessions.length}
      >
        {sessions === null ? (
          <EntitySectionLoading />
        ) : sessions.length === 0 ? (
          <EntitySectionEmpty label="No active sessions." />
        ) : (
          sessions.map((s) => {
            const tone = pressureTone(s.pressure_level);
            return (
              <EntityRow
                key={s.session_id}
                tag={s.pressure_level.toUpperCase()}
                name={s.session_id.slice(0, 12)}
                subtitle={`turn ${s.current_turn} · ${formatTokens(s.tokens_used)} tokens`}
                meta={`${(s.utilization * 100).toFixed(0)}%`}
                onClick={() =>
                  openEntity({
                    kind: "session",
                    corpusId: corpus.id,
                    sessionId: s.session_id,
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

function Stat({ label, value }: { label: string; value: string }) {
  const _ = cn; // keep import
  void _;
  return (
    <div className="border-r border-b border-border-soft [&:nth-child(2n)]:border-r-0 [&:nth-last-child(-n+2)]:border-b-0 px-3 py-2">
      <p className="font-mono text-[0.6875rem] uppercase tracking-[0.05em] text-text-dim">
        {label}
      </p>
      <p className="font-mono text-base font-semibold tabular-nums text-text mt-0.5">
        {value}
      </p>
    </div>
  );
}
