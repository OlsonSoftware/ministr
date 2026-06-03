import { AnimatePresence, motion } from "motion/react";
import { Search } from "lucide-react";
import type { DaemonStatus, SessionDetail } from "../../lib/types";
import { fade } from "../../lib/motion";
import { cn } from "../../lib/utils";
import { DaemonDot } from "../shell/DaemonDot";
import { NumberTicker } from "../ui/number-ticker";
import { SpinePicker } from "./SpinePicker";
import { FacetBar } from "./FacetBar";
import { SessionLayer } from "./SessionLayer";
import { ScopeHeader } from "./ScopeHeader";
import { FACET_BY_ID } from "./facets";
import { useWorkspace, type FacetId } from "./WorkspaceContext";

interface Props {
  status: DaemonStatus | null;
  error?: string | null;
  /** Live sessions, scoped to the spine by the parent. Passed as a prop so the
   *  ⚡ layer renders populated in Storybook (useSessions is Tauri-gated). */
  sessions?: readonly SessionDetail[];
  onOpenSession?: (session: SessionDetail) => void;
  onOpenLogs?: () => void;
  onOpenPalette?: () => void;
  onAddProject?: () => void;
  /** Render the body for a facet (Project spine). Falls back to a scoped
   *  placeholder when omitted (shell-only stories). */
  renderFacet?: (facet: FacetId) => React.ReactNode;
  /** Render the body for the Fleet spine (the collection). Falls back to a
   *  placeholder when omitted. */
  renderFleet?: () => React.ReactNode;
}

/**
 * The integrated workspace shell — the OOUX foundation.
 *
 * One spine (Project | Fleet, selected once in the chrome) + one facet
 * switcher (Ask·Explore·Activity·Tend). Every facet renders beneath a single
 * ScopeHeader that reflects the spine, so switching facets keeps the SAME
 * object in view. This replaces the six sibling destinations + the per-surface
 * `activeCorpusId` re-pick with one shared context.
 *
 * This chunk ships the shell + a scoped placeholder outlet; the facets chunk
 * swaps the shipped surfaces into {@link FACET_BY_ID} and flips `App.tsx`.
 */
export function WorkspaceShell({
  status,
  error = null,
  sessions,
  onOpenSession,
  onOpenLogs,
  onOpenPalette,
  onAddProject,
  renderFacet,
  renderFleet,
}: Props) {
  const { facet, isFleet, corpora } = useWorkspace();

  return (
    <div className="flex flex-col h-full min-h-0 bg-bg">
      {/* Row 1 — the spine chrome (wordmark · spine · vitals · ⌘K · daemon). */}
      <header
        className="flex items-center gap-3 border-b border-border bg-surface px-3 h-12 shrink-0"
        role="banner"
      >
        <div className="ministr-wordmark shrink-0 select-none">ministr</div>
        <div className="h-5 w-px bg-border shrink-0" aria-hidden />
        <SpinePicker onAddProject={onAddProject} />
        <div className="flex-1" />

        {status && (
          <div className="hidden md:flex items-center gap-4 font-mono text-mono-mini text-text-dim">
            <Vital label="mem">
              <NumberTicker
                value={status.memory_mb}
                format={(n) => `${Math.round(n)}MB`}
                className="text-text"
              />
            </Vital>
          </div>
        )}

        <SessionLayer
          sessions={sessions ?? []}
          corpora={corpora}
          onOpenSession={onOpenSession}
        />

        {onOpenPalette && (
          <button
            type="button"
            onClick={onOpenPalette}
            title="Command palette · ⌘K"
            className={cn(
              "hidden sm:inline-flex items-center gap-2 h-8 pl-2.5 pr-2 rounded-md cursor-pointer",
              "border border-border bg-surface text-text-dim",
              "hover:bg-surface-overlay hover:text-text hover:border-border-hover",
              "transition-colors duration-150",
            )}
          >
            <Search className="h-3.5 w-3.5" strokeWidth={2} />
            <kbd className="font-mono text-mono-micro rounded border border-border bg-surface-overlay px-1 py-px">
              ⌘K
            </kbd>
          </button>
        )}

        <div className="h-5 w-px bg-border shrink-0" aria-hidden />
        <DaemonDot status={status} error={error} onOpenLogs={onOpenLogs} />
      </header>

      {isFleet ? (
        /* Fleet spine — the collection view owns the whole body. Picking a
           project there zooms in and reveals the facet bar. */
        <main className="flex-1 min-h-0 overflow-hidden bg-bg" role="main">
          {renderFleet ? renderFleet() : <FleetPlaceholder />}
        </main>
      ) : (
        <>
          {/* Row 2 — the facet switcher (only meaningful on a project). */}
          <FacetBar />

          {/* Facet outlet — always scoped to the spine. ScopeHeader is OUTSIDE
              the cross-fade so the object visibly persists while only the facet
              (the verb) changes. */}
          <main className="flex-1 min-h-0 overflow-hidden bg-bg" role="main">
            <div className="flex flex-col h-full min-h-0">
              <ScopeHeader />
              <div className="flex-1 min-h-0 overflow-auto">
                <AnimatePresence mode="wait">
                  <motion.div
                    key={facet}
                    variants={fade}
                    initial="initial"
                    animate="animate"
                    exit="exit"
                    className="h-full"
                    role="tabpanel"
                  >
                    {renderFacet ? renderFacet(facet) : <FacetOutletPlaceholder />}
                  </motion.div>
                </AnimatePresence>
              </div>
            </div>
          </main>
        </>
      )}
    </div>
  );
}

/**
 * Shell-chunk placeholder. Echoes the facet identity + the shared scope so the
 * "one context / switching keeps context" integration test is visible. The
 * facets chunk replaces this with the shipped surface for each facet.
 */
function FacetOutletPlaceholder() {
  const { facet, isFleet, activeProject } = useWorkspace();
  const meta = FACET_BY_ID[facet];
  const Icon = meta.icon;
  const scope = isFleet
    ? "the fleet"
    : (activeProject?.id ? activeProject.display_name ?? activeProject.id : "the project");

  return (
    <div className="flex h-full min-h-[320px] flex-col items-center justify-center gap-4 px-6 py-12 text-center">
      <span
        className="flex h-16 w-16 items-center justify-center rounded-2xl border border-border bg-surface-overlay text-accent"
        aria-hidden
      >
        <Icon className="h-7 w-7" strokeWidth={1.75} />
      </span>
      <div className="space-y-1.5">
        <h2 className="font-sans text-lg font-semibold text-text">{meta.label}</h2>
        <p className="max-w-sm font-sans text-sm text-text-muted">{meta.blurb}</p>
      </div>
      <p className="max-w-sm font-mono text-mono-mini text-text-dim">
        The {meta.label} facet mounts here — scoped to{" "}
        <span className="text-text-muted">{scope}</span>.
      </p>
    </div>
  );
}

/** Fallback fleet body when no renderFleet is supplied (shell-only stories). */
function FleetPlaceholder() {
  return (
    <div className="flex flex-col h-full min-h-0">
      <ScopeHeader />
      <div className="flex flex-1 min-h-[280px] items-center justify-center px-6 py-12 text-center">
        <p className="max-w-sm font-mono text-mono-mini text-text-dim">
          The Projects collection mounts here — pick a project to zoom into its
          facets.
        </p>
      </div>
    </div>
  );
}

function Vital({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <span className="flex items-baseline gap-1.5">
      <span className="tabular-nums font-semibold">{children}</span>
      <span className="uppercase tracking-[0.08em] text-[0.92em]">{label}</span>
    </span>
  );
}
