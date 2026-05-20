// F2.7 — 4-step onboarding wizard rendered inside CloudPanel.
//
// SOLID layering:
//   - useOnboarding (sibling file) — persistence concern only.
//   - This file — orchestration: which steps are complete, which is
//     current, what each step's action does.
//   - Each step row is a small inline component (StepRow) that just
//     renders a checkbox + title + description + primary action.
//
// Step completeness is derived from props (cloud status, plan,
// corpus count) instead of stored — the wizard always reflects
// reality. Skipping is built in: every step has a "Skip" link that
// just advances the cursor; the user can come back via the same
// wizard if they re-open it.

import { useMemo, useState } from "react";
import { Check, ChevronRight, Loader2, X } from "lucide-react";

import { Button } from "../ui/button";
import { cn } from "../../lib/utils";
import { useOnboarding } from "./useOnboarding";

/**
 * External signals the wizard reads. Pass these in from CloudPanel
 * so the wizard stays a pure presentation layer.
 */
export interface OnboardingSignals {
  /** Cloud bearer token present + endpoint configured. */
  authenticated: boolean;
  /** Plan resolved on the cloud — empty string until usage probes. */
  plan: string | null;
  /** User has at least one corpus registered on the cloud. */
  hasCorpus: boolean;
  /** Whether the user has saved a GitHub App installation ID at
   *  least once (read from a localStorage-mirrored flag set by the
   *  clone dialog). */
  hasGithubAppInstallation: boolean;
}

/**
 * Handlers the wizard invokes on the primary action of each step. The
 * parent (CloudPanel) owns the actual command calls so the wizard
 * doesn't re-implement them.
 */
export interface OnboardingHandlers {
  onSignInGitHub: () => Promise<void> | void;
  onUpgradePro: () => Promise<void> | void;
  onInstallGitHubApp: () => Promise<void> | void;
  onCloneFirstRepo: () => void;
}

interface Step {
  id: "signin" | "plan" | "github-app" | "clone";
  title: string;
  description: string;
  complete: boolean;
  primaryLabel: string;
  primary: () => Promise<void> | void;
}

export function OnboardingWizard({
  signals,
  handlers,
}: {
  signals: OnboardingSignals;
  handlers: OnboardingHandlers;
}) {
  const { dismissed, dismiss } = useOnboarding();
  const [busyId, setBusyId] = useState<Step["id"] | null>(null);

  const steps: Step[] = useMemo(
    () => [
      {
        id: "signin",
        title: "Sign in with GitHub",
        description:
          "We federate sign-in through GitHub. Your bearer token lands in the OS keychain — never on disk.",
        complete: signals.authenticated,
        primaryLabel: "Sign in",
        primary: handlers.onSignInGitHub,
      },
      {
        id: "plan",
        title: "Confirm your plan",
        description:
          "Pick the tier that fits — Pro at $20/mo or Team at $30/seat. Stripe-hosted Checkout, returns here when done.",
        complete: signals.plan === "pro" || signals.plan === "team" || signals.plan === "enterprise",
        primaryLabel: "Pick Pro ($20/mo)",
        primary: handlers.onUpgradePro,
      },
      {
        id: "github-app",
        title: "Install the GitHub App",
        description:
          "Optional. Lets you clone private repos without handing the cloud a personal access token.",
        complete: signals.hasGithubAppInstallation,
        primaryLabel: "Install GitHub App",
        primary: handlers.onInstallGitHubApp,
      },
      {
        id: "clone",
        title: "Clone your first repo",
        description:
          "Drop in a Git URL or pick a local path. Indexing kicks off automatically and the bridge graph lights up.",
        complete: signals.hasCorpus,
        primaryLabel: "Clone a repo",
        primary: handlers.onCloneFirstRepo,
      },
    ],
    [signals, handlers],
  );

  const allComplete = steps.every((s) => s.complete);
  if (dismissed || allComplete) return null;

  const completed = steps.filter((s) => s.complete).length;
  const total = steps.length;

  return (
    <section
      aria-label="Onboarding"
      className="flex flex-col gap-3 rounded-lg border border-accent/40 bg-accent/5 p-4"
    >
      <header className="flex items-center justify-between gap-2">
        <div className="flex flex-col gap-1">
          <h3 className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-accent">
            Get started — {completed} of {total}
          </h3>
          <p className="text-xs text-text-muted">
            Four short steps. Each one is resumable and skippable; come back
            via Settings → Cloud whenever.
          </p>
        </div>
        <button
          type="button"
          onClick={dismiss}
          aria-label="Dismiss onboarding"
          className="rounded-md p-1 text-text-muted hover:bg-surface-overlay hover:text-text transition-colors"
        >
          <X className="size-4" />
        </button>
      </header>

      <ol className="flex flex-col gap-2">
        {steps.map((step, idx) => (
          <StepRow
            key={step.id}
            index={idx + 1}
            step={step}
            busy={busyId === step.id}
            onActivate={async () => {
              setBusyId(step.id);
              try {
                await step.primary();
              } finally {
                setBusyId(null);
              }
            }}
          />
        ))}
      </ol>
    </section>
  );
}

function StepRow({
  index,
  step,
  busy,
  onActivate,
}: {
  index: number;
  step: Step;
  busy: boolean;
  onActivate: () => Promise<void>;
}) {
  return (
    <li
      className={cn(
        "flex items-center gap-3 rounded-md px-3 py-2 transition-colors",
        step.complete
          ? "border border-accent/30 bg-accent/10"
          : "border border-border-soft bg-surface",
      )}
    >
      <span
        className={cn(
          "flex size-6 items-center justify-center rounded-full border text-xs font-semibold",
          step.complete
            ? "border-accent/60 bg-accent/20 text-accent"
            : "border-border-soft text-text-muted",
        )}
        aria-hidden
      >
        {step.complete ? <Check className="size-3.5" /> : index}
      </span>
      <div className="flex flex-1 flex-col">
        <p className={cn("text-sm font-medium", step.complete && "line-through text-text-muted")}>
          {step.title}
        </p>
        <p className="text-xs text-text-muted">{step.description}</p>
      </div>
      {!step.complete && (
        <Button size="sm" onClick={() => void onActivate()} disabled={busy}>
          {busy ? (
            <Loader2 className="size-3.5 animate-spin" />
          ) : (
            <ChevronRight className="size-3.5" />
          )}
          {step.primaryLabel}
        </Button>
      )}
    </li>
  );
}
