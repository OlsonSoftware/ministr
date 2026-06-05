/**
 * FirstRunGuide — first value, INSIDE the workspace (aaa-onboarding).
 *
 * Replaces the old full-screen, exit-the-app wizard with a focused guide that
 * overlays the workspace (the chrome stays visible behind a scrim) and drives
 * straight to value: point at a folder → watch it index live → ask your first
 * question. The new project becomes the spine and the Ask facet opens — the
 * aha moment happens IN the workspace, not in a wizard you leave.
 *
 * `FirstRunGuide` is PURE (state derived from `corpora` + callbacks) so every
 * step renders in Storybook. `FirstRunOverlay` is the thin connector that
 * wires it to the daemon commands + the spine (useWorkspace).
 *
 * Built from the v4 tokens + ui/ atoms — a fresh composition, not the retired
 * wizard. The legacy Setup(PATH) + Connect(MCP) steps relocate to Account/Tend
 * (aaa-onboarding-setup-mcp-relocate).
 */
import { type ReactNode, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AnimatePresence, motion } from "motion/react";
import {
  ArrowRight,
  ArrowUpRight,
  FolderOpen,
  Loader2,
  Search,
  Sparkles,
} from "@/components/ui/icons";

import type { CorpusInfo, DaemonStatus, DetectedProject } from "../../lib/types";
import { corpusLabel } from "../../lib/corpus";
import { popIn, scrim } from "../../lib/motion";
import { overlayScrim } from "../../lib/ui-tokens";
import { cn } from "../../lib/utils";
import { useWorkspace } from "../workspace/WorkspaceContext";
import { useToast } from "../shell/ToastTray";
import { Button } from "../ui/button";
import { Logo } from "../ui/logo";
import { Progress } from "../ui/progress";

/** The boot screen's glowing brand medallion (aaa-logo-boot-signature), reused
 *  here so first-run and the cold-launch boot read as one brand. Tone rides the
 *  border + glow (non-text); the reduced-motion-safe `ministr-pulse` halo makes
 *  it feel alive while a step is "working". */
function HeroMedallion({
  tone = "accent",
  pulse = false,
  children,
}: {
  tone?: "accent" | "success";
  pulse?: boolean;
  children: ReactNode;
}) {
  return (
    <span
      aria-hidden
      className={cn(
        "relative grid h-16 w-16 shrink-0 place-items-center rounded-2xl border bg-surface-overlay shadow-[var(--glow-soft)]",
        tone === "success" ? "border-success/50 text-success" : "border-accent/50 text-accent",
      )}
    >
      {pulse && (
        <span
          aria-hidden
          className="ministr-pulse pointer-events-none absolute inset-0 rounded-2xl"
        />
      )}
      <span className="relative grid place-items-center">{children}</span>
    </span>
  );
}

type FirstRunStep = "welcome" | "indexing" | "ask";

/** Derive the step from the live corpus list — the guide follows the data. */
function deriveStep(corpora: CorpusInfo[]): {
  step: FirstRunStep;
  ready: CorpusInfo | null;
} {
  if (corpora.length === 0) return { step: "welcome", ready: null };
  const anyIndexing = corpora.some(
    (c) => c.status.state === "indexing" || c.status.state === "queued",
  );
  if (anyIndexing) return { step: "indexing", ready: null };
  const ready =
    corpora.find((c) => c.status.state === "idle" && c.files_indexed > 0) ??
    corpora[0];
  return { step: "ask", ready };
}

interface FirstRunGuideProps {
  corpora: CorpusInfo[];
  /** A pick/detect call is in flight. */
  busy?: boolean;
  onPickFolder: () => void;
  onAutoDetect: () => void;
  onAsk: () => void;
  onSkip: () => void;
}

export function FirstRunGuide({
  corpora,
  busy = false,
  onPickFolder,
  onAutoDetect,
  onAsk,
  onSkip,
}: FirstRunGuideProps) {
  const { step, ready } = deriveStep(corpora);

  return (
    <motion.div
      variants={scrim}
      initial="initial"
      animate="animate"
      exit="exit"
      className={cn(overlayScrim, "z-[1200] grid place-items-center p-6")}
    >
      <motion.div
        variants={popIn}
        initial="initial"
        animate="animate"
        exit="exit"
        role="dialog"
        aria-modal="true"
        aria-label="Get started"
        className="w-full max-w-lg overflow-hidden rounded-2xl border border-border border-t-2 border-t-accent/50 bg-surface-raised shadow-2xl"
      >
        {/* Header — wordmark + skip. */}
        <header className="flex items-center justify-between gap-3 border-b border-border bg-surface-raised px-6 h-12">
          <div className="flex items-center gap-2.5 min-w-0">
            {/* The brand's first hello — mark + wordmark lockup. */}
            <Logo className="h-[18px] w-[18px] shrink-0" title="ministr" />
            <span className="ministr-wordmark select-none">ministr</span>
            <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
              Get started
            </span>
          </div>
          <button
            type="button"
            onClick={onSkip}
            className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim hover:text-text cursor-pointer transition-colors duration-150"
          >
            Skip
          </button>
        </header>

        <div className="p-6">
          <AnimatePresence mode="wait">
            <motion.div
              key={step}
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -6 }}
              transition={{ duration: 0.18 }}
            >
              {step === "welcome" && (
                <WelcomeStep
                  busy={busy}
                  onPickFolder={onPickFolder}
                  onAutoDetect={onAutoDetect}
                />
              )}
              {step === "indexing" && <IndexingStep corpora={corpora} />}
              {step === "ask" && ready && (
                <AskStep corpus={ready} onAsk={onAsk} />
              )}
            </motion.div>
          </AnimatePresence>
        </div>
      </motion.div>
    </motion.div>
  );
}

// ── Step 1 — value-first: a branded hero, then point at a folder. ──────────
function WelcomeStep({
  busy,
  onPickFolder,
  onAutoDetect,
}: {
  busy: boolean;
  onPickFolder: () => void;
  onAutoDetect: () => void;
}) {
  return (
    <div className="flex flex-col items-center gap-5 text-center">
      {/* The brand's first hello — the boot medallion. */}
      <HeroMedallion pulse>
        <Logo className="h-7 w-7" gradient />
      </HeroMedallion>

      <div className="space-y-2">
        <h1 className="font-sans text-3xl font-bold leading-tight tracking-[-0.01em] text-text">
          Ask your codebase anything.
        </h1>
        <p className="mx-auto max-w-sm font-sans text-sm text-text-muted leading-relaxed">
          Point ministr at a folder. It indexes locally — code, docs, symbols,
          cross-language bridges — then answers with cited source. No setup
          first; value first.
        </p>
      </div>

      <div className="grid w-full grid-cols-1 sm:grid-cols-2 gap-3">
        <PickAction
          icon={FolderOpen}
          title="Pick a folder"
          hint="Open a system file picker"
          onClick={onPickFolder}
          disabled={busy}
        />
        <PickAction
          icon={Search}
          title="Auto-detect"
          hint="Scan ~/Code, ~/Projects"
          onClick={onAutoDetect}
          disabled={busy}
          loading={busy}
        />
      </div>

      <div className="flex flex-wrap justify-center gap-2 pt-0.5">
        {["Local-only", "Cited answers", "MCP-ready"].map((t) => (
          <span
            key={t}
            className="inline-flex items-center gap-1 rounded-full border border-border-soft bg-surface-sunken px-2.5 py-1 font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim"
          >
            <Sparkles className="h-3 w-3 text-accent" strokeWidth={2.5} />
            {t}
          </span>
        ))}
      </div>
    </div>
  );
}

// ── Step 2 — live indexing as an alive moment. ─────────────────────────────
function IndexingStep({ corpora }: { corpora: CorpusInfo[] }) {
  return (
    <div className="flex flex-col items-center gap-5 text-center">
      {/* A scanning medallion — the accent pulse signals live work. */}
      <HeroMedallion pulse>
        <Search className="h-7 w-7" strokeWidth={2} />
      </HeroMedallion>

      <div className="space-y-2">
        <h1 className="font-sans text-3xl font-bold leading-tight tracking-[-0.01em] text-text">
          Reading your code…
        </h1>
        <p className="mx-auto max-w-sm font-sans text-sm text-text-muted leading-relaxed">
          Scanning every file once, extracting symbols + cross-language links,
          embedding the chunks. You can ask the moment it&apos;s ready.
        </p>
      </div>

      <ul className="w-full space-y-2.5 text-left">
        {corpora.map((c) => {
          const indexing = c.status.state === "indexing" ? c.status : null;
          const pct =
            indexing && indexing.files_total > 0
              ? Math.min(
                  100,
                  Math.round((indexing.files_done / indexing.files_total) * 100),
                )
              : c.status.state === "queued"
                ? 0
                : 100;
          return (
            <li
              key={c.id}
              className="rounded-lg border border-border bg-surface px-3.5 py-2.5 shadow-sm"
            >
              <div className="flex items-center justify-between gap-2 mb-1.5">
                <span className="font-mono text-sm font-semibold text-text truncate">
                  {corpusLabel(c)}
                </span>
                <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim tabular-nums shrink-0">
                  {indexing
                    ? `${indexing.files_done.toLocaleString()} / ${indexing.files_total.toLocaleString()}`
                    : c.status.state === "queued"
                      ? "Queued"
                      : "Ready"}
                </span>
              </div>
              <Progress value={pct} tone={pct >= 100 ? "success" : "accent"} />
            </li>
          );
        })}
      </ul>
    </div>
  );
}

// ── Step 3 — a triumphant "ready", then the first ask. ─────────────────────
function AskStep({
  corpus,
  onAsk,
}: {
  corpus: CorpusInfo;
  onAsk: () => void;
}) {
  return (
    <div className="flex flex-col items-center gap-5 text-center">
      {/* Ready — a success-toned medallion lands the win. */}
      <HeroMedallion tone="success">
        <Sparkles className="h-7 w-7" strokeWidth={2} />
      </HeroMedallion>

      <div className="space-y-2">
        <h1 className="font-sans text-3xl font-bold leading-tight tracking-[-0.01em] text-text">
          <span className="font-mono">{corpusLabel(corpus)}</span> is ready.
        </h1>
        <p className="mx-auto max-w-sm font-sans text-sm text-text-muted leading-relaxed">
          Ask your first question — the answer lands right here in the workspace.
        </p>
      </div>

      {/* A divided vital readout — the index, at a glance. */}
      <div className="flex w-full items-stretch divide-x divide-border overflow-hidden rounded-lg border border-border bg-surface">
        <Vital value={corpus.files_indexed} label="files" />
        <Vital value={corpus.sections_count} label="sections" />
        <Vital value={corpus.symbols_count} label="symbols" />
      </div>

      <Button size="lg" onClick={onAsk} className="w-full justify-center">
        Ask your first question
        <ArrowRight className="h-4 w-4" strokeWidth={2} />
      </Button>
    </div>
  );
}

function Vital({ value, label }: { value: number; label: string }) {
  return (
    <div className="flex flex-1 flex-col items-center gap-0.5 px-3 py-2.5">
      <span className="font-mono text-base font-semibold tabular-nums text-text">
        {value.toLocaleString()}
      </span>
      <span className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
        {label}
      </span>
    </div>
  );
}

/** A premium pick action — a glowing accent icon tile that lights on hover, a
 *  lift to the raised tier, and a hover-revealed accent arrow (the Ask-hero
 *  starter-card pattern). */
function PickAction({
  icon: Icon,
  title,
  hint,
  onClick,
  disabled,
  loading,
}: {
  icon: typeof FolderOpen;
  title: string;
  hint: string;
  onClick: () => void;
  disabled?: boolean;
  loading?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "group relative flex flex-col items-start gap-2 rounded-xl p-4 text-left cursor-pointer",
        "border border-border bg-surface shadow-sm",
        "hover:bg-surface-raised hover:border-accent hover:-translate-y-0.5 hover:shadow-md",
        "disabled:opacity-50 disabled:cursor-not-allowed disabled:hover:translate-y-0",
        "transition-[transform,box-shadow,border-color,background-color] duration-150 ease-out",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
      )}
    >
      <span className="grid h-9 w-9 place-items-center rounded-lg border border-accent/40 bg-accent/10 text-accent transition-shadow duration-150 group-hover:shadow-[var(--glow-soft)]">
        {loading ? (
          <Loader2 className="h-[18px] w-[18px] animate-spin" strokeWidth={2} />
        ) : (
          <Icon className="h-[18px] w-[18px]" strokeWidth={2} />
        )}
      </span>
      <span className="font-sans text-sm font-semibold text-text">{title}</span>
      <span className="font-mono text-mono-mini text-text-dim">{hint}</span>
      <ArrowUpRight
        className="absolute right-3 top-3 h-4 w-4 text-text-dim opacity-0 transition-opacity duration-150 group-hover:text-accent group-hover:opacity-100"
        strokeWidth={2.25}
      />
    </button>
  );
}

// ─────────────────────────────────────────────────────────────────────────
// Connector — wires the pure guide to the daemon + the spine. Rendered INSIDE
// WorkspaceProvider so a picked/indexed project can become the spine and the
// first ask can open the Ask facet.

export function FirstRunOverlay({
  status,
  onRefresh,
  onDone,
}: {
  status: DaemonStatus;
  onRefresh: () => void;
  onDone: () => void;
}) {
  const { selectProject, setFacet } = useWorkspace();
  const { toast } = useToast();
  const [busy, setBusy] = useState(false);

  async function pickFolder() {
    setBusy(true);
    try {
      const res = await invoke<{ corpus_id: string } | null>(
        "add_project_dialog",
      );
      if (res) {
        onRefresh();
        selectProject(res.corpus_id); // the new project becomes the spine
      }
    } catch (e) {
      toast("Couldn’t add project", { detail: String(e), tone: "danger" });
    } finally {
      setBusy(false);
    }
  }

  async function autoDetect() {
    setBusy(true);
    try {
      const detected = await invoke<DetectedProject[]>("detect_projects");
      if (detected.length === 0) {
        toast("No projects found", {
          detail: "Try Pick a folder",
          tone: "info",
        });
        return;
      }
      const ids = await invoke<string[]>("register_projects_batch", {
        paths: detected.map((d) => d.path),
      });
      onRefresh();
      if (ids[0]) selectProject(ids[0]);
    } catch (e) {
      toast("Scan failed", { detail: String(e), tone: "danger" });
    } finally {
      setBusy(false);
    }
  }

  async function finish(openAsk: boolean) {
    try {
      await invoke("dismiss_onboarding");
    } catch {
      /* non-fatal — local-only flag */
    }
    if (openAsk) setFacet("ask");
    onDone();
  }

  return (
    <FirstRunGuide
      corpora={status.corpora}
      busy={busy}
      onPickFolder={pickFolder}
      onAutoDetect={autoDetect}
      onAsk={() => finish(true)}
      onSkip={() => finish(false)}
    />
  );
}
