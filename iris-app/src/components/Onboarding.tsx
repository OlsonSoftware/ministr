import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  CircleDot,
  FolderOpen,
  ArrowRight,
  ArrowLeft,
  CheckCircle2,
  Search,
  Sparkles,
  Layers,
  Gauge,
  Zap,
  X,
} from "lucide-react";
import { Button } from "./ui/button";
import { cn } from "../lib/utils";
import type { DetectedProject } from "../lib/types";

interface OnboardingProps {
  onDismiss: () => void;
}

type Step = "welcome" | "detect" | "done";

export function Onboarding({ onDismiss }: OnboardingProps) {
  const [step, setStep] = useState<Step>("welcome");
  const [detected, setDetected] = useState<DetectedProject[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [scanning, setScanning] = useState(false);
  const [importing, setImporting] = useState(false);
  const [importedCount, setImportedCount] = useState(0);

  useEffect(() => {
    if (step === "detect") scanProjects();
  }, [step]);

  async function scanProjects() {
    setScanning(true);
    try {
      const projects = await invoke<DetectedProject[]>("detect_projects");
      setDetected(projects);
      setSelected(new Set(projects.map((p) => p.path)));
    } catch (err) {
      console.error("[iris] detect_projects error:", err);
    } finally {
      setScanning(false);
    }
  }

  function toggleProject(path: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      next.has(path) ? next.delete(path) : next.add(path);
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
      console.error("[iris] register_projects_batch error:", err);
    } finally {
      setImporting(false);
    }
  }

  async function addManually() {
    await invoke("add_project_dialog");
    await dismiss();
  }

  async function dismiss() {
    await invoke("dismiss_onboarding");
    onDismiss();
  }

  return (
    <div className="relative flex h-full items-center justify-center overflow-hidden bg-bg p-6">
      {/* Ambient iris wash */}
      <div
        className="pointer-events-none absolute inset-0"
        aria-hidden="true"
        style={{
          background: `
            radial-gradient(900px 600px at 20% -10%, color-mix(in srgb, var(--color-accent) 14%, transparent), transparent 65%),
            radial-gradient(700px 500px at 100% 100%, color-mix(in srgb, var(--color-accent) 10%, transparent), transparent 70%)
          `,
        }}
      />

      <div className="relative w-full max-w-xl iris-fade-in">
        {step === "welcome" && <Welcome onContinue={() => setStep("detect")} onManual={addManually} onSkip={dismiss} />}
        {step === "detect" && (
          <Detect
            scanning={scanning}
            detected={detected}
            selected={selected}
            importing={importing}
            onBack={() => setStep("welcome")}
            onToggle={toggleProject}
            onToggleAll={toggleAll}
            onManual={addManually}
            onImport={importSelected}
          />
        )}
        {step === "done" && <Done count={importedCount} onDismiss={dismiss} />}
      </div>
    </div>
  );
}

function Logo({ large = false }: { large?: boolean }) {
  return (
    <div
      className={cn(
        "relative grid place-items-center rounded-2xl text-[var(--color-accent-fg-on)]",
        "bg-gradient-to-br from-accent to-[color-mix(in_srgb,var(--color-accent)_50%,#c4b5fd)]",
        "shadow-[0_8px_32px_var(--color-accent-ring),inset_0_1px_0_rgb(255_255_255/0.25)]",
        large ? "h-16 w-16" : "h-12 w-12",
      )}
    >
      <CircleDot
        className={large ? "h-8 w-8" : "h-6 w-6"}
        strokeWidth={2.5}
      />
    </div>
  );
}

function Welcome({
  onContinue,
  onManual,
  onSkip,
}: {
  onContinue: () => void;
  onManual: () => void;
  onSkip: () => void;
}) {
  const features = [
    {
      icon: Search,
      title: "Semantic search",
      body: "Embedding-based retrieval across docs, code, and claims.",
    },
    {
      icon: Layers,
      title: "Session-aware",
      body: "Dedup delivered content, deliver deltas when files change.",
    },
    {
      icon: Zap,
      title: "Predictive prefetch",
      body: "Pre-warm what the agent is about to ask for next.",
    },
    {
      icon: Gauge,
      title: "Budget awareness",
      body: "Track token usage, flag pressure, recommend evictions.",
    },
  ];

  return (
    <div className="rounded-2xl border border-border/70 bg-surface/85 backdrop-blur-sm p-8 shadow-[var(--shadow-lg)]">
      <div className="flex flex-col items-center text-center">
        <Logo large />
        <div className="mt-5 flex items-center gap-2">
          <span className="iris-wordmark text-2xl">iris</span>
          <Sparkles className="h-4 w-4 text-accent opacity-80" />
        </div>
        <p className="mt-3 max-w-md text-sm leading-relaxed text-text-muted">
          A context cache for your LLM agent. iris tracks what it has delivered,
          pre-warms what's next, and flags budget pressure — locally, with no API keys.
        </p>
      </div>

      <div className="mt-7 grid grid-cols-2 gap-3">
        {features.map((f) => (
          <div
            key={f.title}
            className="rounded-lg border border-border/60 bg-surface-overlay/40 p-3"
          >
            <div className="flex items-center gap-2">
              <div className="grid h-6 w-6 place-items-center rounded-md bg-[var(--color-accent-soft)] text-accent">
                <f.icon className="h-3.5 w-3.5" />
              </div>
              <p className="text-[13px] font-semibold text-text">{f.title}</p>
            </div>
            <p className="mt-1.5 text-xs leading-snug text-text-muted">
              {f.body}
            </p>
          </div>
        ))}
      </div>

      <div className="mt-7 space-y-2">
        <Button className="w-full" size="lg" onClick={onContinue}>
          <Search className="h-3.5 w-3.5" />
          Scan for projects
          <ArrowRight className="h-3.5 w-3.5" />
        </Button>
        <Button variant="outline" size="lg" className="w-full" onClick={onManual}>
          <FolderOpen className="h-3.5 w-3.5" />
          Pick a folder manually
        </Button>
      </div>

      <button
        onClick={onSkip}
        className="mx-auto mt-4 flex items-center gap-1 text-[11px] text-text-dim hover:text-text-muted cursor-pointer"
      >
        Skip for now
        <ArrowRight className="h-3 w-3" />
      </button>
    </div>
  );
}

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
    <div className="rounded-2xl border border-border/70 bg-surface/85 backdrop-blur-sm p-7 shadow-[var(--shadow-lg)]">
      <div className="flex items-start gap-3">
        <Logo />
        <div className="flex-1">
          <h2 className="text-base font-semibold text-text">Detected projects</h2>
          <p className="mt-0.5 text-xs text-text-dim">
            Scanning{" "}
            <span className="font-mono">~/Code · ~/Projects · ~/Developer · ~/src</span>{" "}
            for <span className="font-mono">.iris.toml</span>
          </p>
        </div>
      </div>

      <div className="mt-5 min-h-[220px]">
        {scanning ? (
          <div className="flex flex-col items-center justify-center gap-3 py-10">
            <div className="iris-spin h-8 w-8 rounded-full border-2 border-border border-t-accent" />
            <p className="text-xs text-text-muted">Scanning…</p>
          </div>
        ) : detected.length === 0 ? (
          <div className="flex flex-col items-center justify-center gap-2 py-10 text-center">
            <div className="grid h-12 w-12 place-items-center rounded-xl bg-surface-overlay text-text-dim">
              <FolderOpen className="h-5 w-5" />
            </div>
            <p className="text-sm font-medium text-text">No iris projects found</p>
            <p className="max-w-xs text-xs text-text-dim">
              Drop a <span className="font-mono">.iris.toml</span> into any
              project root, or add a folder manually.
            </p>
          </div>
        ) : (
          <>
            <div className="mb-2 flex items-center justify-between text-[11px]">
              <button
                onClick={onToggleAll}
                className="text-accent hover:underline cursor-pointer"
              >
                {selected.size === detected.length ? "Deselect all" : "Select all"}
              </button>
              <span className="font-mono text-text-dim">
                {selected.size} / {detected.length}
              </span>
            </div>

            <div className="max-h-72 space-y-1 overflow-y-auto pr-1">
              {detected.map((project) => {
                const isSelected = selected.has(project.path);
                return (
                  <label
                    key={project.path}
                    className={cn(
                      "flex items-center gap-3 rounded-lg border px-3 py-2 cursor-pointer transition-all duration-120",
                      isSelected
                        ? "border-[var(--color-accent-ring)] bg-[var(--color-accent-soft)] shadow-[0_0_0_3px_var(--color-accent-soft)]"
                        : "border-border/60 hover:border-border-hover hover:bg-surface-overlay/60",
                    )}
                  >
                    <input
                      type="checkbox"
                      checked={isSelected}
                      onChange={() => onToggle(project.path)}
                      className="h-3.5 w-3.5 accent-accent cursor-pointer"
                    />
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-[13px] font-semibold text-text">
                        {project.name}
                      </div>
                      <div className="truncate font-mono text-[11px] text-text-dim">
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
          <ArrowLeft className="h-3.5 w-3.5" />
          Back
        </Button>
        <div className="flex-1" />
        <Button variant="outline" size="sm" onClick={onManual}>
          <FolderOpen className="h-3.5 w-3.5" />
          Add manually
        </Button>
        <Button size="sm" onClick={onImport} disabled={importing}>
          {importing
            ? "Importing…"
            : selected.size > 0
              ? `Add ${selected.size} project${selected.size !== 1 ? "s" : ""}`
              : "Skip"}
          {!importing && <ArrowRight className="h-3.5 w-3.5" />}
        </Button>
      </div>
    </div>
  );
}

function Done({ count, onDismiss }: { count: number; onDismiss: () => void }) {
  return (
    <div className="rounded-2xl border border-border/70 bg-surface/85 backdrop-blur-sm p-8 shadow-[var(--shadow-lg)] text-center">
      <div className="flex justify-center">
        <div className="relative grid h-16 w-16 place-items-center rounded-2xl bg-success/15 text-success shadow-[0_8px_32px_color-mix(in_srgb,#34d399_35%,transparent),inset_0_1px_0_rgb(255_255_255/0.15)]">
          <CheckCircle2 className="h-8 w-8" strokeWidth={2.25} />
        </div>
      </div>
      <h2 className="mt-5 text-lg font-semibold text-text">You're all set</h2>
      <p className="mx-auto mt-2 max-w-sm text-sm text-text-muted">
        {count === 0
          ? "iris is ready. Add projects anytime from the dashboard or the tray."
          : count === 1
            ? "1 project is being indexed. You can add more anytime from the dashboard."
            : `${count} projects are being indexed. You can add more anytime from the dashboard.`}
      </p>
      <Button className="mt-6" size="lg" onClick={onDismiss}>
        Open dashboard
        <ArrowRight className="h-3.5 w-3.5" />
      </Button>
    </div>
  );
}

export function OnboardingSkipBadge({ onClick }: { onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      aria-label="Dismiss onboarding"
      className="inline-flex h-7 w-7 items-center justify-center rounded-md text-text-dim hover:bg-surface-overlay hover:text-text cursor-pointer"
    >
      <X className="h-3.5 w-3.5" />
    </button>
  );
}
