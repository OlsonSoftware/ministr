import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowLeft,
  ArrowRight,
  FolderOpen,
  Plus,
  X,
} from "lucide-react";
import { Button } from "./ui/button";
import { H1 } from "./ui/heading";
import { cn } from "../lib/utils";
import type { DetectedProject } from "../lib/types";

interface OnboardingProps {
  onDismiss: () => void;
}

type Step = "welcome" | "detect" | "done";

const STEPS: { key: Step; n: number }[] = [
  { key: "welcome", n: 1 },
  { key: "detect", n: 2 },
  { key: "done", n: 3 },
];

export function Onboarding({ onDismiss }: OnboardingProps) {
  const [step, setStep] = useState<Step>("welcome");
  const [detected, setDetected] = useState<DetectedProject[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [scanning, setScanning] = useState(false);
  const [showScanning, setShowScanning] = useState(false);
  const [importing, setImporting] = useState(false);
  const [importedCount, setImportedCount] = useState(0);
  const scanTimer = useRef<number | null>(null);

  useEffect(() => {
    if (step === "detect") scanProjects();
  }, [step]);

  // Debounce the SCANNING_ indicator: only show after 300ms of waiting.
  useEffect(() => {
    if (scanning) {
      scanTimer.current = window.setTimeout(() => setShowScanning(true), 300);
    } else {
      if (scanTimer.current !== null) {
        clearTimeout(scanTimer.current);
        scanTimer.current = null;
      }
      setShowScanning(false);
    }
    return () => {
      if (scanTimer.current !== null) clearTimeout(scanTimer.current);
    };
  }, [scanning]);

  async function scanProjects() {
    setScanning(true);
    try {
      const projects = await invoke<DetectedProject[]>("detect_projects");
      setDetected(projects);
      setSelected(new Set(projects.map((p) => p.path)));
    } catch (err) {
      console.error("[ministr] detect_projects error:", err);
    } finally {
      setScanning(false);
    }
  }

  function toggleProject(path: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  function toggleAll() {
    setSelected(
      selected.size === detected.length
        ? new Set()
        : new Set(detected.map((p) => p.path)),
    );
  }

  async function importSelected() {
    if (selected.size === 0) {
      await dismiss();
      return;
    }
    setImporting(true);
    try {
      const paths = Array.from(selected);
      const ids = await invoke<string[]>("register_projects_batch", { paths });
      setImportedCount(ids.length);
      setStep("done");
    } catch (err) {
      console.error("[ministr] register_projects_batch error:", err);
    } finally {
      setImporting(false);
    }
  }

  async function addManually(advanceToDone = false) {
    try {
      await invoke("add_project_dialog");
    } catch {
      /* ignore */
    }
    if (advanceToDone) {
      setImportedCount((c) => c + 1);
      setStep("done");
    } else {
      await dismiss();
    }
  }

  async function addAnotherFromDone() {
    try {
      await invoke("add_project_dialog");
      setImportedCount((c) => c + 1);
    } catch {
      /* ignore */
    }
  }

  async function dismiss() {
    await invoke("dismiss_onboarding");
    onDismiss();
  }

  return (
    <div className="flex h-full items-center justify-center bg-bg p-6">
      <div className="w-full max-w-xl">
        <div className="border border-border-soft bg-surface shadow-lg">
          <StepIndicator current={step} />
          <div className="p-8">
            {step === "welcome" && (
              <Welcome
                onContinue={() => setStep("detect")}
                onManual={() => addManually(false)}
              />
            )}
            {step === "detect" && (
              <Detect
                scanning={showScanning}
                detected={detected}
                selected={selected}
                importing={importing}
                onBack={() => setStep("welcome")}
                onToggle={toggleProject}
                onToggleAll={toggleAll}
                onManual={() => addManually(true)}
                onImport={importSelected}
              />
            )}
            {step === "done" && (
              <Done
                count={importedCount}
                onDismiss={dismiss}
                onAddAnother={addAnotherFromDone}
              />
            )}
            {step !== "done" && (
              <div className="mt-6 flex items-center justify-end">
                <button
                  onClick={dismiss}
                  className="inline-flex items-center gap-1 font-sans text-xs tracking-[0.05em] text-text-dim hover:text-text cursor-pointer"
                >
                  Skip for now
                  <ArrowRight className="h-3 w-3" strokeWidth={2.5} />
                </button>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

// ─── STEP INDICATOR ──────────────────────────────────────────────────────

function StepIndicator({ current }: { current: Step }) {
  return (
    <div className="flex items-center justify-center gap-2 border-b-2 border-border bg-surface-overlay px-4 py-2">
      {STEPS.map((s, i) => {
        const isCurrent = s.key === current;
        const isPast =
          STEPS.findIndex((x) => x.key === current) > i;
        return (
          <div key={s.key} className="flex items-center gap-2">
            <span
              className={cn(
                "inline-flex items-center gap-1 border border-border-soft px-2 py-0.5 font-mono text-xs font-bold uppercase tracking-[0.05em] transition-none",
                isCurrent
                  ? "bg-accent text-[var(--color-accent-fg-on)] shadow-sm"
                  : isPast
                    ? "bg-surface text-text"
                    : "bg-surface text-text-dim",
              )}
            >
              <span className="tabular-nums">{s.n}</span>
              <span>{s.key}</span>
            </span>
            {i < STEPS.length - 1 && (
              <span className="font-mono text-xs text-text-dim">·</span>
            )}
          </div>
        );
      })}
    </div>
  );
}

// ─── WELCOME ─────────────────────────────────────────────────────────────

function Welcome({
  onContinue,
  onManual,
}: {
  onContinue: () => void;
  onManual: () => void;
}) {
  return (
    <div>
      <div className="text-center">
        <span
          className="ministr-wordmark"
          style={{ fontSize: "48px", borderBottomWidth: "6px" }}
        >
          ministr
        </span>
        <p className="font-serif italic text-base text-text-dim mt-4">
          Code intelligence for LLM agents.
        </p>
        <p className="mt-4 max-w-md mx-auto font-sans text-sm leading-relaxed text-text-muted">
          ministr indexes your codebase. Survey, find symbols, follow
          references, map cross-language bridges — locally, with no API keys.
        </p>
      </div>

      <div className="mt-6 flex flex-wrap items-center justify-center gap-x-3 gap-y-1">
        {["Survey", "Symbols", "References", "Bridge"].map((f, i) => (
          <span
            key={f}
            className="inline-flex items-center gap-3 font-serif text-sm font-bold text-text"
          >
            {f}
            {i < 3 && (
              <span className="font-mono text-text-dim">·</span>
            )}
          </span>
        ))}
      </div>

      <p className="mt-5 text-center font-sans text-xs text-text-dim">
        Press{" "}
        <kbd
          className="border border-border-soft bg-surface-overlay px-1 py-0 font-mono text-mono-mini text-text-muted rounded-sm"
        >
          ⌘K
        </kbd>{" "}
        anywhere to jump.
      </p>

      <div className="mt-7 flex flex-col gap-2">
        <Button className="w-full" size="lg" onClick={onContinue}>
          Scan for projects
          <ArrowRight className="h-3.5 w-3.5" strokeWidth={2} />
        </Button>
        <Button
          variant="outline"
          size="lg"
          className="w-full"
          onClick={onManual}
        >
          <FolderOpen className="h-3.5 w-3.5" strokeWidth={2} />
          Pick a folder manually
        </Button>
      </div>
    </div>
  );
}

// ─── DETECT ──────────────────────────────────────────────────────────────

function Detect({
  scanning,
  detected,
  selected,
  importing,
  onBack,
  onToggle,
  onToggleAll,
  onManual,
  onImport,
}: {
  scanning: boolean;
  detected: DetectedProject[];
  selected: Set<string>;
  importing: boolean;
  onBack: () => void;
  onToggle: (path: string) => void;
  onToggleAll: () => void;
  onManual: () => void;
  onImport: () => void;
}) {
  return (
    <div>
      <div>
        <H1>Detected projects</H1>
        <p className="mt-1 font-serif text-sm italic text-text-dim">
          Scanning ~/Code · ~/Projects · ~/Developer · ~/src for .ministr.toml
        </p>
      </div>

      <div className="mt-5 min-h-[220px]">
        {scanning ? (
          <div className="flex flex-col items-center justify-center gap-3 py-10">
            <p className="font-serif text-base italic text-text-muted">
              Scanning<span className="ministr-blink">_</span>
            </p>
          </div>
        ) : detected.length === 0 ? (
          <div className="flex flex-col items-center justify-center gap-2 py-10 text-center">
            <div className="grid h-12 w-12 place-items-center border border-border-soft bg-surface-overlay text-text-muted">
              <FolderOpen className="h-5 w-5" strokeWidth={2} />
            </div>
            <p className="font-serif text-lg font-bold text-text">
              No projects found
            </p>
            <p className="max-w-xs font-serif text-sm italic text-text-dim">
              Drop a{" "}
              <span className="font-mono not-italic">.ministr.toml</span> into any
              project root, or add a folder manually.
            </p>
          </div>
        ) : (
          <>
            <div className="mb-2 flex items-center justify-between">
              <button
                onClick={onToggleAll}
                className="font-sans text-sm font-medium text-text-muted hover:text-text border-b border-transparent hover:border-text cursor-pointer"
              >
                {selected.size === detected.length
                  ? "Deselect all"
                  : "Select all"}
              </button>
              <span className="font-mono text-xs tabular-nums text-text-dim">
                {selected.size} / {detected.length}
              </span>
            </div>

            <div className="max-h-72 space-y-0 overflow-y-auto">
              {detected.map((project) => {
                const isSelected = selected.has(project.path);
                return (
                  <label
                    key={project.path}
                    className={cn(
                      "relative flex items-center gap-3 border border-border-soft px-3 py-2 cursor-pointer transition-none -mt-[1px] first:mt-0",
                      isSelected
                        ? "bg-surface-overlay border-accent text-text"
                        : "bg-surface text-text-muted hover:bg-surface-overlay hover:text-text hover:border-border",
                    )}
                  >
                    <input
                      type="checkbox"
                      checked={isSelected}
                      onChange={() => onToggle(project.path)}
                      className="h-3.5 w-3.5 accent-accent cursor-pointer"
                    />
                    <div className="min-w-0 flex-1">
                      <div className="truncate font-mono text-sm font-semibold">
                        {project.name}
                      </div>
                      <div className="truncate font-mono text-xs text-text-dim">
                        {project.path}
                      </div>
                    </div>
                  </label>
                );
              })}
            </div>
          </>
        )}
      </div>

      <div className="mt-5 flex items-center gap-2">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft className="h-3.5 w-3.5" strokeWidth={2} />
          Back
        </Button>
        <div className="flex-1" />
        <Button variant="outline" size="sm" onClick={onManual}>
          <FolderOpen className="h-3.5 w-3.5" strokeWidth={2} />
          Add manually
        </Button>
        <Button size="sm" onClick={onImport} disabled={importing}>
          {importing
            ? "Importing…"
            : selected.size > 0
              ? `Add ${selected.size}`
              : "Skip"}
          {!importing && (
            <ArrowRight className="h-3.5 w-3.5" strokeWidth={2} />
          )}
        </Button>
      </div>
    </div>
  );
}

// ─── DONE ────────────────────────────────────────────────────────────────

function Done({
  count,
  onDismiss,
  onAddAnother,
}: {
  count: number;
  onDismiss: () => void;
  onAddAnother: () => void;
}) {
  const tips: { key: string; description: string }[] = [
    {
      key: "Ask a question",
      description: "Natural-language Q&A grounded in your codebase.",
    },
    {
      key: "g e",
      description: "Explore — sections, symbols, and bridges in one place.",
    },
    {
      key: "⌘K",
      description: "Command palette — jump anywhere instantly.",
    },
  ];

  return (
    <div>
      <div className="text-center">
        <H1>Ready</H1>
        <p className="mx-auto mt-2 max-w-sm font-sans text-sm text-text-muted">
          {count === 0
            ? "ministr is ready. Add projects anytime from the dashboard or the tray."
            : count === 1
              ? "1 project indexing."
              : `${count} projects indexing.`}
        </p>
      </div>

      {/* §1 Try this */}
      <div className="mt-6 border border-border-soft bg-surface-overlay">
        <div className="flex items-baseline gap-3 border-b border-border-soft px-3 py-2">
          <span className="font-serif text-base font-normal text-text-dim tabular-nums shrink-0 w-6">
            §1
          </span>
          <h3 className="font-serif text-base font-bold text-text">
            Try this
          </h3>
        </div>
        <ul className="divide-y divide-border-soft">
          {tips.map((t, i) => (
            <li
              key={t.key}
              className="flex items-baseline gap-3 px-3 py-2"
            >
              <span className="font-serif text-sm text-text-dim w-5 text-right tabular-nums shrink-0">
                {i + 1}
              </span>
              <span className="inline-flex items-center font-sans text-sm font-semibold text-text shrink-0 border-b-2 border-accent pb-px">
                {t.key}
              </span>
              <span className="font-sans text-sm text-text-muted">
                {t.description}
              </span>
            </li>
          ))}
        </ul>
      </div>

      <div className="mt-6 flex flex-col gap-2">
        <Button className="w-full" size="lg" onClick={onDismiss}>
          Ask your first question
          <ArrowRight className="h-3.5 w-3.5" strokeWidth={2.5} />
        </Button>
        <Button
          variant="outline"
          size="lg"
          className="w-full"
          onClick={onAddAnother}
        >
          <Plus className="h-3.5 w-3.5" strokeWidth={2.5} />
          Add another project
        </Button>
      </div>
    </div>
  );
}

export function OnboardingSkipBadge({ onClick }: { onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      aria-label="Dismiss onboarding"
      className="inline-flex h-7 w-7 items-center justify-center border-2 border-border text-text-dim hover:bg-surface-overlay hover:text-text cursor-pointer transition-none"
    >
      <X className="h-3.5 w-3.5" strokeWidth={2.5} />
    </button>
  );
}
