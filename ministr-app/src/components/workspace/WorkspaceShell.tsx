import { AnimatePresence, motion } from "motion/react";
import { Search } from "lucide-react";
import type { DaemonStatus } from "../../lib/types";
import { fade } from "../../lib/motion";
import { cn } from "../../lib/utils";
import { DaemonDot } from "../shell/DaemonDot";
import { NumberTicker } from "../ui/number-ticker";
import { SpinePicker } from "./SpinePicker";
import { FacetBar } from "./FacetBar";
import { ScopeHeader } from "./ScopeHeader";
import { FACET_BY_ID } from "./facets";
import { useWorkspace } from "./WorkspaceContext";

interface Props {
  status: DaemonStatus | null;
  error?: string | null;
  /** Lifted so the shell stays pure in Storybook — live App passes
   *  `useSessions().sessions.length`. */
  sessionCount?: number;
  onOpenLogs?: () => void;
  onOpenPalette?: () => void;
  onAddProject?: () => void;
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
  sessionCount = 0,
  onOpenLogs,
  onOpenPalette,
  onAddProject,
}: Props) {
  const { facet } = useWorkspace();

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
            <Vital label="sessions">
              <NumberTicker value={sessionCount} flashOnChange className="text-text" />
            </Vital>
            <Vital label="mem">
              <NumberTicker
                value={status.memory_mb}
                format={(n) => `${Math.round(n)}MB`}
                className="text-text"
              />
            </Vital>
          </div>
        )}

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

      {/* Row 2 — the facet switcher. */}
      <FacetBar />

      {/* Facet outlet — always scoped to the spine. ScopeHeader is OUTSIDE the
          cross-fade so the object visibly persists while only the facet
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
                <FacetOutletPlaceholder />
              </motion.div>
            </AnimatePresence>
          </div>
        </div>
      </main>
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
