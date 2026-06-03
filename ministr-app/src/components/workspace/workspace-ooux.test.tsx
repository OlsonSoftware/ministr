/**
 * aaa-workspace EPIC — the OOUX Definition-of-Done as executable invariants.
 *
 * AAA-VISION.md collapses six sibling destinations into ONE object-centric
 * workspace. These three integration tests pin the properties that "done"
 * actually means, so a regression (like the Activity facet ignoring the spine,
 * fixed in 107ff42) fails the suite instead of shipping:
 *
 *   1. ONE CONTEXT      — the spine (project) is picked once; switching facets
 *                         never re-picks it, and facets scope to it.
 *   2. NO DUPLICATE     — a single session-card renderer serves both the board
 *      RENDERERS          (expand) and the inspector (inspect) modes.
 *   3. GROWS BY FACET/   — the IA is 4 facet VERBS on one spine NOUN, reachable
 *      COMMAND             from the command palette — not 6 task-destinations.
 */
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";

import type { CorpusInfo, DaemonStatus, SessionDetail } from "../../lib/types";
import {
  WorkspaceProvider,
  useWorkspace,
  FACET_IDS,
} from "./WorkspaceContext";
import { SessionsSurface } from "../surfaces/SessionsSurface";
import { SessionCard } from "../ui/session-card";
import { CommandPalette } from "../chrome/CommandPalette";

// ── Fixtures ────────────────────────────────────────────────────────────────

const session = (over: Partial<SessionDetail> & { session_id: string }): SessionDetail => ({
  corpus_id: "proj-a",
  current_turn: 5,
  delivered_count: 12,
  tokens_used: 40_000,
  tokens_remaining: 160_000,
  utilization: 0.2,
  pressure_level: "normal",
  total_deliveries: 12,
  cumulative_tokens_delivered: 40_000,
  total_tokens_saved: 8_000,
  total_evictions: 0,
  total_compressions: 1,
  dedup_hits: 3,
  compression_ratio: 0.5,
  client_name: "claude-code",
  ...over,
});

// Two sessions on proj-a + one on proj-b — the cross-corpus shape that exposes
// a facet that fails to scope to the spine.
const { MOCK_SESSIONS } = vi.hoisted(() => {
  const mk = (sid: string, corpus: string): SessionDetail => ({
    session_id: sid,
    corpus_id: corpus,
    current_turn: 5,
    delivered_count: 12,
    tokens_used: 40_000,
    tokens_remaining: 160_000,
    utilization: 0.2,
    pressure_level: "normal",
    total_deliveries: 12,
    cumulative_tokens_delivered: 40_000,
    total_tokens_saved: 8_000,
    total_evictions: 0,
    total_compressions: 1,
    dedup_hits: 3,
    compression_ratio: 0.5,
    client_name: "claude-code",
  });
  return {
    MOCK_SESSIONS: [
      mk("sess_aaa111", "proj-a"),
      mk("sess_aaa222", "proj-a"),
      mk("sess_bbb999", "proj-b"),
    ],
  };
});

vi.mock("../../hooks/useSessions", () => ({
  useSessions: () => ({
    sessions: MOCK_SESSIONS,
    byId: new Map(MOCK_SESSIONS.map((s) => [s.session_id, s])),
    samples: new Map(),
    freshIds: new Set(),
    loaded: true,
  }),
}));

vi.mock("../../hooks/useEntityPanel", () => ({
  useEntityPanel: () => ({ openEntity: () => {} }),
}));

const corpus = (id: string): CorpusInfo => ({
  id,
  display_name: id,
  paths: [`/Users/alrik/Code/${id}`],
  status: { state: "idle" },
  files_indexed: 100,
  sections_count: 500,
  embeddings_count: 500,
  active_sessions: 1,
  symbols_count: 200,
});

const status: DaemonStatus = {
  version: "0.2.1",
  uptime_secs: 1000,
  memory_mb: 300,
  model: "jina-code-v2",
  model_dimension: 768,
  corpora: [corpus("proj-a"), corpus("proj-b")],
  total_sessions: 3,
};

// ── 1. ONE CONTEXT ──────────────────────────────────────────────────────────

describe("one context", () => {
  function Probe() {
    const w = useWorkspace();
    return (
      <div>
        <span data-testid="active">{w.activeProjectId ?? "fleet"}</span>
        <span data-testid="facet">{w.facet}</span>
        <button onClick={() => w.setFacet("activity")}>to-activity</button>
        <button onClick={() => w.setFacet("tend")}>to-tend</button>
        <button onClick={() => w.selectProject("proj-b")}>pick-b</button>
      </div>
    );
  }

  it("keeps the spine project fixed while switching facets (picked once)", () => {
    render(
      <WorkspaceProvider
        corpora={status.corpora}
        initialSpine={{ kind: "project", id: "proj-a" }}
        initialFacet="ask"
      >
        <Probe />
      </WorkspaceProvider>,
    );

    expect(screen.getByTestId("active")).toHaveTextContent("proj-a");
    expect(screen.getByTestId("facet")).toHaveTextContent("ask");

    fireEvent.click(screen.getByText("to-activity"));
    expect(screen.getByTestId("facet")).toHaveTextContent("activity");
    // The verb moved; the noun did not.
    expect(screen.getByTestId("active")).toHaveTextContent("proj-a");

    fireEvent.click(screen.getByText("to-tend"));
    expect(screen.getByTestId("facet")).toHaveTextContent("tend");
    expect(screen.getByTestId("active")).toHaveTextContent("proj-a");

    // Re-picking the spine is an explicit act, not a side effect of a facet.
    fireEvent.click(screen.getByText("pick-b"));
    expect(screen.getByTestId("active")).toHaveTextContent("proj-b");
    expect(screen.getByTestId("facet")).toHaveTextContent("tend");
  });

  it("scopes the Activity facet to the spine project (regression: 107ff42)", () => {
    const { rerender } = render(
      <SessionsSurface status={status} activeCorpusId="proj-a" />,
    );
    // proj-a has 2 sessions; the proj-b session must NOT leak in.
    expect(screen.getByText(/2 live agent sessions/i)).toBeInTheDocument();
    expect(screen.queryByText("sess_bbb999")).not.toBeInTheDocument();

    // On the Fleet (no spine project) the whole fleet shows — all three.
    rerender(<SessionsSurface status={status} activeCorpusId={null} />);
    expect(screen.getByText(/3 live agent sessions/i)).toBeInTheDocument();
    expect(screen.getByText("sess_bbb999")).toBeInTheDocument();
  });
});

// ── 2. NO DUPLICATE RENDERERS ────────────────────────────────────────────────

describe("no duplicate renderers", () => {
  it("renders the same session-card component in both expand and inspect modes", () => {
    const s = session({ session_id: "sess_unify42" });

    const expand = render(
      <SessionCard
        session={s}
        interaction="expand"
        corpus={corpus("proj-a")}
        onToggle={() => {}}
        onOpenInspector={() => {}}
      />,
    );
    expect(
      within(expand.container).getByText("sess_unify42"),
    ).toBeInTheDocument();
    expand.unmount();

    const inspect = render(
      <SessionCard
        session={s}
        interaction="inspect"
        corpora={[corpus("proj-a")]}
        onOpenInspector={() => {}}
      />,
    );
    // Same renderer, the other interaction mode — no second card component.
    expect(
      within(inspect.container).getByText("sess_unify42"),
    ).toBeInTheDocument();
  });
});

// ── 3. GROWS BY FACET / COMMAND ──────────────────────────────────────────────

describe("grows by facet, not destination", () => {
  it("models exactly four facet verbs over one spine", () => {
    // Four facets applied to one noun — not six sibling destinations.
    expect([...FACET_IDS]).toEqual(["ask", "explore", "activity", "tend"]);
  });

  it("surfaces facet navigation through the command palette", () => {
    render(
      <WorkspaceProvider
        corpora={status.corpora}
        initialSpine={{ kind: "project", id: "proj-a" }}
      >
        <CommandPalette
          open
          onClose={() => {}}
          corpora={status.corpora}
          activeCorpusId="proj-a"
          onNavigate={() => {}}
          onSelectCorpus={() => {}}
          onAddProject={() => {}}
          onOpenLogs={() => {}}
          onReindexActive={() => {}}
          onCycleTheme={() => {}}
        />
      </WorkspaceProvider>,
    );

    // The palette is the keyboard-first connective tissue: every facet verb is
    // reachable as a command (the IA grows by command, not by new destination).
    // Exact labels (not substrings) so "Tasks"/"ask the project" don't match.
    const dialog = screen.getByRole("dialog");
    for (const label of ["Ask", "Explore", "Activity", "Tend"]) {
      expect(within(dialog).getAllByText(label).length).toBeGreaterThan(0);
    }
  });
});
