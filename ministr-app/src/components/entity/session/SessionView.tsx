import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AlertOctagon } from "lucide-react";
import type { Entity } from "../../../hooks/useEntityPanel";
import { useEntityPanel } from "../../../hooks/useEntityPanel";
import { useSession } from "../../../hooks/useSessions";
import { useSessionActivity } from "../../../hooks/useSessionActivity";
import { endedSessionSeed } from "../../../hooks/useSessionHistory";
import type { CorpusInfo, SessionDetail } from "../../../lib/types";
import { EntitySection } from "../EntitySection";
import { EntityRow } from "../EntityRow";
import { EmptyState } from "../../ui/empty-state";
import { Button } from "../../ui/button";
import { SessionHeroStrip } from "./SessionHeroStrip";
import { SessionCodeTouchedSection } from "./SessionCodeTouchedSection";
import { ActivityTimeline } from "./ActivityTimeline";
import { SessionLineageSection } from "./SessionLineageSection";
import { SessionTokenAdvanced } from "./SessionTokenAdvanced";

interface Props {
  entity: Extract<Entity, { kind: "session" }>;
}

/**
 * SessionView — the deep per-session drawer. Orchestrator only: owns
 * the hooks (one shared sessions subscription + an activity poll) and
 * composes the hero strip + numbered chapters. Sections take
 * already-fetched data as props so they stay pure.
 *
 * Order (code-intelligence first, token economics demoted):
 *   Hero strip
 *   §1 Code touched
 *   §2 Activity
 *   §3 Lineage (when present)
 *   §4 Project
 *   §5 Token usage (collapsed)
 *
 * Liveness: prefers live store data; falls back to the caller-supplied
 * `seed`, then the 24h ended-session history, so reopening an ended
 * session still renders its last-known state instead of a blank panel.
 */
export function SessionView({ entity }: Props) {
  const { corpusId, sessionId } = entity;
  const { openEntity } = useEntityPanel();

  const live = useSession(sessionId);
  const activity = useSessionActivity(sessionId);

  // Cross-section filter state — §1 file-row clicks set this, which
  // flows down to §2 as a search-input default. Bidirectional: §2's
  // search clear bubbles back up so §1 can drop highlighting.
  const [targetFilter, setTargetFilter] = useState("");

  const session: SessionDetail | null = useMemo(
    () => live.session ?? entity.seed ?? endedSessionSeed(sessionId),
    [live.session, entity.seed, sessionId],
  );

  // Corpus detail for the §Project row — one-shot; cheap and static-ish.
  const [corpus, setCorpus] = useState<CorpusInfo | null>(null);
  useEffect(() => {
    let cancelled = false;
    invoke<CorpusInfo[]>("list_corpora")
      .then((list) => {
        if (!cancelled) {
          setCorpus(list.find((c) => c.id === corpusId) ?? null);
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [corpusId]);

  const openSession = (s: SessionDetail) =>
    openEntity({
      kind: "session",
      corpusId: s.corpus_id,
      sessionId: s.session_id,
      seed: s,
    });

  // No data at all.
  if (!session) {
    if (!live.loaded) {
      return (
        <div className="flex flex-col gap-4">
          <div className="border border-border-soft bg-surface px-4 py-3.5">
            <p className="font-sans text-base italic text-text-dim">
              Loading session<span className="ministr-blink">_</span>
            </p>
          </div>
        </div>
      );
    }
    return (
      <EmptyState
        icon={AlertOctagon}
        title={live.error ? "Can't reach the daemon" : "Session not found"}
        hint={
          live.error
            ? "The ministr daemon isn't responding. The panel retries automatically."
            : "This session has ended and is no longer in recent history."
        }
        action={
          live.error ? (
            <Button variant="outline" size="sm" onClick={activity.refresh}>
              Retry now
            </Button>
          ) : undefined
        }
      />
    );
  }

  const hasLineage = !!live.parent || live.children.length > 0;
  let ch = 0;

  return (
    <div className="flex flex-col gap-4">
      <SessionHeroStrip
        session={session}
        isLive={live.isLive}
        stale={live.stale}
        fresh={live.fresh}
        parent={live.parent}
        childCount={live.children.length}
        onOpenParent={
          live.parent ? () => openSession(live.parent as SessionDetail) : undefined
        }
      />

      <SessionCodeTouchedSection
        chapter={(ch += 1)}
        events={activity.events}
        loading={activity.loading}
        onFilterFile={setTargetFilter}
      />

      <ActivityTimeline
        chapter={(ch += 1)}
        events={activity.events}
        loading={activity.loading}
        error={activity.error}
        flashSince={activity.flashSince}
        targetFilter={targetFilter}
        onTargetFilterChange={setTargetFilter}
      />

      {hasLineage && (
        <SessionLineageSection
          chapter={(ch += 1)}
          session={session}
          parent={live.parent}
          children={live.children}
          onOpen={openSession}
        />
      )}

      <EntitySection chapter={(ch += 1)} title="Project">
        <EntityRow
          tag="project"
          name={corpus?.display_name ?? corpusId}
          subtitle={corpus?.paths[0]}
          meta={
            corpus ? `${corpus.sections_count.toLocaleString()} §` : undefined
          }
          onClick={
            corpus ? () => openEntity({ kind: "corpus", corpus }) : undefined
          }
        />
      </EntitySection>

      <SessionTokenAdvanced
        chapter={(ch += 1)}
        session={session}
        samples={live.samples}
        ended={!live.isLive}
      />
    </div>
  );
}
