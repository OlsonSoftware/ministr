import { Fragment } from "react";
import {
  Check,
  Layers,
  ListFilter,
  PenLine,
  ScanSearch,
  ShieldCheck,
  type LucideIcon,
} from "lucide-react";
import { motion } from "motion/react";
import { fadeRise } from "../../../lib/motion";
import { cn } from "../../../lib/utils";
import { StatusDot } from "../../ui/status-dot";
import type { AskPhaseName } from "./internals";

interface Props {
  phase: AskPhaseName;
  /** When true the status block renders the cache-hit short-circuit. */
  cached?: boolean;
}

/**
 * The in-flight Ask status — a command-deck LIVE PIPELINE.
 *
 * Instead of a single opaque "Thinking…" spinner, this narrates every stage
 * the agent actually runs: a 5-step rail (Analyze → Retrieve → Rerank →
 * Synthesize → Verify) where the current stage glows + pulses, completed
 * stages are accent-filled with a check, upcoming stages are muted, and the
 * connectors fill as progress advances. Below the rail, an aria-live line
 * narrates what's happening right now — surfacing ministr's signature
 * retrieval pipeline as a premium live experience.
 *
 * Liveness is a11y-safe: the designed StatusDot pulse (reduced-motion aware)
 * + a static medallion glow, never an ad-hoc spinner.
 */
type Stage = {
  phase: Extract<
    AskPhaseName,
    "analyzing" | "retrieving" | "reranking" | "synthesizing" | "verifying"
  >;
  label: string;
  icon: LucideIcon;
  /** Human narration of what the agent is doing in this stage. */
  detail: string;
};

const PIPELINE: Stage[] = [
  {
    phase: "analyzing",
    label: "Analyze",
    icon: ScanSearch,
    detail: "planning the query & sub-questions",
  },
  {
    phase: "retrieving",
    label: "Retrieve",
    icon: Layers,
    detail: "hybrid dense + sparse search over the index",
  },
  {
    phase: "reranking",
    label: "Rerank",
    icon: ListFilter,
    detail: "cross-encoder reranking the candidates",
  },
  {
    phase: "synthesizing",
    label: "Synthesize",
    icon: PenLine,
    detail: "writing a cited answer from the sources",
  },
  {
    phase: "verifying",
    label: "Verify",
    icon: ShieldCheck,
    detail: "checking every claim against the sources",
  },
];

export function AskStatus({ phase, cached = false }: Props) {
  if (cached && phase === "done") {
    return (
      <motion.div
        variants={fadeRise}
        initial="initial"
        animate="animate"
        role="status"
        className="flex items-center gap-2.5 rounded-xl border border-accent/50 bg-accent-soft px-4 py-2.5 shadow-sm"
      >
        <StatusDot tone="accent" pulse="live" size="md" />
        <span className="font-mono text-mono-mini font-semibold uppercase tracking-[0.06em] text-text">
          From cache
        </span>
        <span className="font-sans text-sm text-text-dim">
          we already had this one
        </span>
      </motion.div>
    );
  }

  const active = PIPELINE.findIndex((s) => s.phase === phase);
  if (active < 0) return null; // idle / done / error render nothing here

  const current = PIPELINE[active];

  return (
    <motion.div
      variants={fadeRise}
      initial="initial"
      animate="animate"
      className="relative rounded-xl border border-border bg-surface-raised px-4 py-3.5 shadow-sm"
    >
      {/* Lit top edge — the deck's signature accent hairline. */}
      <span
        aria-hidden
        className="pointer-events-none absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-accent/50 to-transparent"
      />

      {/* The pipeline rail — decorative; the narration line below is the
          accessible status. */}
      <div aria-hidden className="flex items-start">
        {PIPELINE.map((stage, i) => {
          const state =
            i < active ? "done" : i === active ? "active" : "pending";
          return (
            <Fragment key={stage.phase}>
              <Step stage={stage} state={state} />
              {i < PIPELINE.length - 1 && <Connector filled={i < active} />}
            </Fragment>
          );
        })}
      </div>

      {/* The live narration — what the agent is doing right now. */}
      <div
        role="status"
        aria-live="polite"
        className="mt-3 flex items-center gap-2 border-t border-border-soft pt-3"
      >
        <StatusDot tone="accent" pulse="live" size="md" />
        <span className="font-sans text-sm font-semibold text-text">
          {current.label}
        </span>
        <span className="min-w-0 truncate font-sans text-sm text-text-dim">
          — {current.detail}
        </span>
        <span className="flex-1" />
        <span className="shrink-0 font-mono text-mono-mini tabular-nums text-text-dim">
          {active + 1}/{PIPELINE.length}
        </span>
      </div>
    </motion.div>
  );
}

/** One pipeline stage — an icon chip + a label beneath it. Done stages show a
 *  check; the active stage glows; pending stages are muted. */
function Step({
  stage,
  state,
}: {
  stage: Stage;
  state: "done" | "active" | "pending";
}) {
  const Icon = stage.icon;
  return (
    <div className="flex shrink-0 flex-col items-center gap-1.5">
      <span
        className={cn(
          "grid h-9 w-9 place-items-center rounded-xl border transition-colors duration-300",
          state === "active" &&
            "border-accent/60 bg-accent-soft text-accent shadow-[var(--glow-soft)]",
          state === "done" && "border-accent/40 bg-accent-soft text-accent",
          state === "pending" && "border-border bg-surface text-text-dim",
        )}
      >
        {state === "done" ? (
          <Check className="h-4 w-4" strokeWidth={2.5} />
        ) : (
          <Icon className="h-[18px] w-[18px]" strokeWidth={2} />
        )}
      </span>
      {/* Raw string (not cn): tailwind-merge would otherwise collapse the
          custom `text-mono-micro` font-size against the `text-text-*` color,
          dropping the size — same reason MetricTile keeps its label inline. */}
      <span
        className={
          "hidden font-mono text-mono-micro uppercase tracking-[0.06em] @min-[420px]/page:block " +
          (state === "pending" ? "text-text-dim" : "text-text-muted")
        }
      >
        {stage.label}
      </span>
    </div>
  );
}

/** The rail segment between two stages; fills accent once the left stage is
 *  done. Top margin lands it on the 36px chip's vertical centre. */
function Connector({ filled }: { filled: boolean }) {
  return (
    <span
      className={cn(
        "mt-[17px] h-0.5 flex-1 rounded-full transition-colors duration-300",
        filled ? "bg-accent" : "bg-border",
      )}
    />
  );
}
