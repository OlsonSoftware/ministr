/**
 * Onboarding — first-launch 4-step wizard.
 *
 * 1. Setup: the OS-identical screen. Confirms the native installer
 *    (Apple .pkg / Windows NSIS / Linux .deb·.rpm·.AppImage) put the
 *    CLI on PATH; `fix_path` repairs it in one click if not.
 * 2. Pick a project: auto-detect via `detect_projects` or open the folder
 *    picker. Multi-select adds many at once.
 * 3. Index it: real progress driven by `indexing_progress_events`. User
 *    can "continue in background" once any project transitions out of
 *    pending into running.
 * 4. Connect your AI tool: placeholder for the MCP wizard. Real impl
 *    lands in M3 (Settings → AI assistants is the same panel reused).
 */
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowRight,
  Check,
  FolderOpen,
  Loader2,
  Search,
  Sparkles,
  Terminal,
  TriangleAlert,
} from "lucide-react";

import { cn } from "../lib/utils";
import type { CorpusInfo, DetectedProject } from "../lib/types";
import { useIndexingProgress } from "../hooks/useIndexingProgress";
import { useDaemonStatus } from "../hooks/useDaemonStatus";
import { Progress } from "./ui/progress";
import { Button } from "./ui/button";
import { AiAssistantsPanel } from "./surfaces/AiAssistantsPanel";
import { formatEtaBare } from "../lib/format";

type Step = "setup" | "pick" | "index" | "connect";

/** Mirror of the Rust `SetupStatus` (commands.rs::setup_status). */
interface SetupStatus {
  cli_on_path: boolean;
  cli_path: string | null;
  data_dir: string;
  version: string;
}

interface OnboardingProps {
  onDismiss: () => void;
}

export function Onboarding({ onDismiss }: OnboardingProps) {
  const [step, setStep] = useState<Step>("setup");
  // IDs of corpora the user added during this onboarding run. Step 2
  // watches only these, not every corpus the daemon has registered.
  const [watchIds, setWatchIds] = useState<string[]>([]);

  return (
    <div className="flex h-full flex-col bg-bg text-text">
      <header className="flex items-center justify-between gap-4 px-8 py-4 shrink-0">
        <div className="flex items-center gap-4">
          <span className="ministr-wordmark">ministr</span>
          <StepIndicator step={step} />
        </div>
        <Button
          variant="ghost"
          size="sm"
          onClick={async () => {
            await invoke("dismiss_onboarding");
            onDismiss();
          }}
          className="text-text-dim hover:text-text"
        >
          Skip
          <ArrowRight className="h-3 w-3" strokeWidth={2.5} />
        </Button>
      </header>

      <main className="flex-1 min-h-0 overflow-y-auto">
        <div className="mx-auto max-w-3xl px-8 py-6">
          {step === "setup" && (
            <StepSetup onContinue={() => setStep("pick")} />
          )}
          {step === "pick" && (
            <StepPick
              onIndexed={(ids) => {
                setWatchIds(ids);
                setStep("index");
              }}
            />
          )}
          {step === "index" && (
            <StepIndex
              watchIds={watchIds}
              onContinue={() => setStep("connect")}
            />
          )}
          {step === "connect" && (
            <StepConnect
              onDone={async () => {
                await invoke("dismiss_onboarding");
                onDismiss();
              }}
            />
          )}
        </div>
      </main>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────
// Step indicator

function StepIndicator({ step }: { step: Step }) {
  const items: { key: Step; label: string }[] = [
    { key: "setup", label: "Setup" },
    { key: "pick", label: "Pick" },
    { key: "index", label: "Index" },
    { key: "connect", label: "Connect" },
  ];
  const currentIdx = items.findIndex((i) => i.key === step);
  return (
    <ol className="flex items-center gap-2 font-mono text-mono-mini font-semibold uppercase tracking-[0.08em]">
      {items.map((item, idx) => {
        const isActive = idx === currentIdx;
        const isDone = idx < currentIdx;
        return (
          <li key={item.key} className="flex items-center gap-2">
            <span
              className={cn(
                "inline-flex h-5 w-5 items-center justify-center border",
                isActive
                  ? "border-accent text-accent bg-surface"
                  : isDone
                    ? "border-accent bg-accent text-accent-fg-on"
                    : "border-border-soft text-text-dim",
              )}
            >
              {isDone ? (
                <Check className="h-3 w-3" strokeWidth={3} />
              ) : (
                idx + 1
              )}
            </span>
            <span className={isActive ? "text-text" : "text-text-dim"}>
              {item.label}
            </span>
            {idx < items.length - 1 && (
              <span className="h-px w-4 bg-border-soft" aria-hidden="true" />
            )}
          </li>
        );
      })}
    </ol>
  );
}

// ─────────────────────────────────────────────────────────────────────────
// Step 1 — Setup
//
// The one screen that is byte-identical on macOS, Windows, and Linux.
// Whatever native installer the user ran (Apple .pkg, Windows NSIS,
// Linux .deb/.rpm/.AppImage) only laid down the app + put the CLI on
// PATH; this is the unified, branded confirmation that it worked — with
// a one-click repair when it didn't.

function StepSetup({ onContinue }: { onContinue: () => void }) {
  const [status, setStatus] = useState<SetupStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [fixing, setFixing] = useState(false);

  async function refresh() {
    setError(null);
    try {
      setStatus(await invoke<SetupStatus>("setup_status"));
    } catch (err) {
      console.error("[ministr] setup_status error:", err);
      setError(String(err));
    }
  }

  useEffect(() => {
    void refresh();
  }, []);

  async function fixPath() {
    setFixing(true);
    setError(null);
    try {
      await invoke<string>("fix_path");
      await refresh();
    } catch (err) {
      console.error("[ministr] fix_path error:", err);
      setError(String(err));
    } finally {
      setFixing(false);
    }
  }

  const ready = status?.cli_on_path === true;

  return (
    <div>
      <p className="font-mono text-mono-mini font-semibold uppercase tracking-[0.08em] text-accent mb-3">
        Step 1 of 4 · Setup
      </p>
      <h1 className="text-display text-text">
        Welcome to
        <br />
        <span className="text-text-dim">ministr.</span>
      </h1>
      <p className="font-sans text-base italic text-text-muted mt-4 max-w-xl leading-relaxed">
        The installer placed the app and wired the{" "}
        <span className="font-mono not-italic text-text">ministr</span> command
        onto your PATH. Same result on macOS, Windows, and Linux — here's the
        confirmation.
      </p>

      <div className="mt-8 border border-border bg-surface">
        <header className="flex items-center justify-between gap-2 border-b-2 border-border bg-surface-overlay px-4 py-2">
          <h2 className="font-mono text-mono-mini font-semibold uppercase tracking-[0.08em] text-text">
            Install status
          </h2>
          {status && (
            <span className="font-mono text-mono-mini text-text-dim">
              v{status.version}
            </span>
          )}
        </header>

        <div className="divide-y divide-border-soft">
          <StatusRow
            ok={ready}
            pending={status === null && error === null}
            title="ministr CLI on PATH"
            detail={
              // Blank while the status is still loading; only surface
              // "not found" once we actually have a (negative) result.
              status === null
                ? undefined
                : (status.cli_path ?? "not found — click Fix PATH")
            }
          />
          <StatusRow
            ok={status !== null}
            pending={status === null && error === null}
            title="Data directory"
            detail={status?.data_dir}
          />
        </div>

        {!ready && status !== null && (
          <footer className="flex items-center justify-between gap-2 border-t-2 border-border px-4 py-2">
            <span className="font-mono text-mono-mini text-text-dim">
              CLI not resolvable from this app
            </span>
            <Button size="sm" onClick={fixPath} disabled={fixing}>
              {fixing && (
                <Loader2 className="h-3 w-3 animate-spin" strokeWidth={2} />
              )}
              Fix PATH
            </Button>
          </footer>
        )}
      </div>

      {error && (
        <p className="mt-3 font-mono text-mono-mini text-danger">{error}</p>
      )}

      <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 mt-10">
        <Capability
          title="Local-only"
          hint="Indexing happens on your machine. No code leaves it."
        />
        <Capability
          title="One installer"
          hint="Identical setup on macOS, Windows, and Linux."
        />
        <Capability
          title="MCP-ready"
          hint="The same daemon serves your editor's AI agent."
        />
      </div>

      <div className="mt-8 flex items-center gap-2 justify-end">
        <Button size="lg" onClick={onContinue}>
          {ready ? "Continue" : "Continue anyway"}
          <ArrowRight className="h-4 w-4" strokeWidth={2} />
        </Button>
      </div>
    </div>
  );
}

function StatusRow({
  ok,
  pending,
  title,
  detail,
}: {
  ok: boolean;
  pending: boolean;
  title: string;
  detail?: string;
}) {
  const statusLabel = pending ? "Loading" : ok ? "OK" : "Warning";
  return (
    <div className="flex items-start gap-3 px-4 py-2.5">
      <span className="mt-0.5 shrink-0" role="img" aria-label={statusLabel}>
        <span className="sr-only">{statusLabel}</span>
        {pending ? (
          <Loader2
            className="h-4 w-4 text-text-dim animate-spin"
            strokeWidth={2}
            aria-hidden="true"
          />
        ) : ok ? (
          <Check
            className="h-4 w-4 text-accent"
            strokeWidth={3}
            aria-hidden="true"
          />
        ) : (
          <TriangleAlert
            className="h-4 w-4 text-danger"
            strokeWidth={2.5}
            aria-hidden="true"
          />
        )}
      </span>
      <div className="flex-1 min-w-0">
        <div className="font-mono text-sm font-semibold text-text">{title}</div>
        {detail && (
          <div className="font-mono text-mono-mini text-text-dim truncate">
            {detail}
          </div>
        )}
      </div>
      <Terminal
        className="h-3.5 w-3.5 text-text-dim shrink-0 mt-1"
        strokeWidth={2}
        aria-hidden="true"
      />
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────
// Step 2 — Pick

function StepPick({ onIndexed }: { onIndexed: (ids: string[]) => void }) {
  const [detected, setDetected] = useState<DetectedProject[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [scanning, setScanning] = useState(false);
  const [showScanning, setShowScanning] = useState(false);
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const scanTimer = useRef<number | null>(null);

  // Debounce the scanning indicator so flash-fast scans don't blink.
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

  async function autoDetect() {
    setError(null);
    setScanning(true);
    try {
      const projects = await invoke<DetectedProject[]>("detect_projects");
      setDetected(projects);
      setSelected(new Set(projects.map((p) => p.path)));
    } catch (err) {
      console.error("[ministr] detect_projects error:", err);
      setError(String(err));
      setDetected([]);
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
    if (!detected) return;
    setSelected(
      selected.size === detected.length
        ? new Set()
        : new Set(detected.map((p) => p.path)),
    );
  }

  async function importSelected() {
    if (selected.size === 0) return;
    setImporting(true);
    setError(null);
    try {
      const paths = Array.from(selected);
      const ids = await invoke<string[]>("register_projects_batch", { paths });
      onIndexed(ids);
    } catch (err) {
      console.error("[ministr] register_projects_batch error:", err);
      setError(String(err));
      setImporting(false);
    }
  }

  async function pickFolder() {
    setError(null);
    try {
      // The dialog flow registers internally; we don't get the new id back.
      // Empty watch list is fine — step 2 falls back to the daemon's full
      // corpus list when watchIds is empty.
      await invoke("add_project_dialog");
      onIndexed([]);
    } catch (err) {
      console.error("[ministr] add_project_dialog error:", err);
      setError(String(err));
    }
  }

  return (
    <div>
      <p className="font-mono text-mono-mini font-semibold uppercase tracking-[0.08em] text-accent mb-3">
        Step 2 of 4 · Project
      </p>
      <h1 className="text-display text-text">
        Ask your codebase
        <br />
        <span className="text-text-dim">anything.</span>
      </h1>
      <p className="font-sans text-base italic text-text-muted mt-4 max-w-xl leading-relaxed">
        Pick a folder. ministr indexes it locally — code, docs, symbols,
        cross-language bridges — then answers questions about it with cited
        source.
      </p>

      {!detected && (
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 mt-8">
          <PrimaryAction
            icon={FolderOpen}
            title="Pick a folder"
            hint="Open a system file picker."
            onClick={pickFolder}
            disabled={importing}
          />
          <PrimaryAction
            icon={Search}
            title="Auto-detect projects"
            hint="Scan common dev dirs (~/Code, ~/Projects)."
            onClick={autoDetect}
            disabled={scanning || importing}
            loading={showScanning}
          />
        </div>
      )}

      {detected && (
        <div className="mt-6 border border-border bg-surface">
          <header className="flex items-center justify-between gap-2 border-b-2 border-border bg-surface-overlay px-4 py-2">
            <h2 className="font-mono text-mono-mini font-semibold uppercase tracking-[0.08em] text-text">
              {detected.length === 0
                ? "No projects detected"
                : `Detected ${detected.length} project${detected.length === 1 ? "" : "s"}`}
            </h2>
            {detected.length > 0 && (
              <button
                onClick={toggleAll}
                className={cn(
                  "font-mono text-mono-mini font-semibold uppercase tracking-[0.08em]",
                  "text-text-dim hover:text-text cursor-pointer transition-colors duration-150 ease-out",
                )}
              >
                {selected.size === detected.length
                  ? "Deselect all"
                  : "Select all"}
              </button>
            )}
          </header>

          {detected.length === 0 ? (
            <div className="px-4 py-6">
              <p className="font-sans text-sm italic text-text-dim mb-3">
                Nothing in the usual places. Try Pick a folder.
              </p>
              <Button
                variant="outline"
                size="sm"
                onClick={() => setDetected(null)}
              >
                Back
              </Button>
            </div>
          ) : (
            <>
              <ul className="divide-y divide-border-soft max-h-[320px] overflow-y-auto">
                {detected.map((p) => {
                  const isSelected = selected.has(p.path);
                  return (
                    <li key={p.path}>
                      <label
                        className={cn(
                          "flex items-start gap-3 px-4 py-2.5 cursor-pointer transition-colors duration-150 ease-out",
                          "hover:bg-surface-overlay",
                          isSelected && "bg-surface-overlay",
                        )}
                      >
                        <input
                          type="checkbox"
                          checked={isSelected}
                          onChange={() => toggleProject(p.path)}
                          className="mt-1 h-4 w-4 accent-accent shrink-0 cursor-pointer"
                        />
                        <div className="flex-1 min-w-0">
                          <div className="font-mono text-sm font-semibold text-text truncate">
                            {p.name}
                          </div>
                          <div className="font-mono text-mono-mini text-text-dim truncate">
                            {p.path}
                          </div>
                        </div>
                      </label>
                    </li>
                  );
                })}
              </ul>
              <footer className="flex items-center justify-between gap-2 border-t-2 border-border px-4 py-2">
                <span className="font-mono text-mono-mini text-text-dim">
                  {selected.size} selected
                </span>
                <div className="flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setDetected(null)}
                    disabled={importing}
                  >
                    Back
                  </Button>
                  <Button
                    size="sm"
                    onClick={importSelected}
                    disabled={importing || selected.size === 0}
                  >
                    {importing && (
                      <Loader2 className="h-3 w-3 animate-spin" strokeWidth={2} />
                    )}
                    Index {selected.size} project
                    {selected.size === 1 ? "" : "s"}
                  </Button>
                </div>
              </footer>
            </>
          )}
        </div>
      )}

      {error && (
        <p className="mt-3 font-mono text-mono-mini text-danger">{error}</p>
      )}

      <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 mt-10">
        <Capability
          title="Local-only"
          hint="Indexing happens on your machine. No code leaves it."
        />
        <Capability
          title="Cited answers"
          hint="Every claim links back to the section it came from."
        />
        <Capability
          title="MCP-ready"
          hint="The same daemon serves your editor's AI agent."
        />
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────
// Step 3 — Index

function StepIndex({
  watchIds,
  onContinue,
}: {
  watchIds: string[];
  onContinue: () => void;
}) {
  const { status } = useDaemonStatus();
  const progress = useIndexingProgress();

  // Filter the daemon's corpus list down to "the ones we just added", or
  // fall back to all corpora when add_project_dialog gave us no IDs.
  const watched: CorpusInfo[] = (() => {
    if (!status) return [];
    if (watchIds.length > 0) {
      return status.corpora.filter((c) => watchIds.includes(c.id));
    }
    return status.corpora;
  })();

  const anyComplete = watched.some(
    (c) => c.status.state === "idle" && c.files_indexed > 0,
  );
  const allComplete =
    watched.length > 0 &&
    watched.every((c) => c.status.state === "idle" && c.files_indexed > 0);

  return (
    <div>
      <p className="font-mono text-mono-mini font-semibold uppercase tracking-[0.08em] text-accent mb-3">
        Step 3 of 4 · Indexing
      </p>
      <h1 className="text-display text-text">
        {allComplete ? "All set." : "Reading your code…"}
      </h1>
      <p className="font-sans text-base italic text-text-muted mt-4 max-w-xl leading-relaxed">
        ministr scans every file once, extracts symbols + cross-language
        links, and embeds the chunks for retrieval. You can continue in
        the background as soon as the first project is ready.
      </p>

      <ul className="mt-8 space-y-3">
        {watched.length === 0 && (
          <li className="font-sans italic text-sm text-text-dim">
            Waiting for the daemon to register your project…
          </li>
        )}
        {watched.map((c) => {
          const ev = progress[c.id];
          const indexing = c.status.state === "indexing";
          const filesDone = ev?.files_done ?? 0;
          const filesTotal = ev?.files_total ?? 0;
          const pct = filesTotal > 0 ? (filesDone / filesTotal) * 100 : 0;
          const done = c.status.state === "idle" && c.files_indexed > 0;
          const eta = ev?.estimated_remaining_secs;
          return (
            <li
              key={c.id}
              className="border border-border bg-surface px-4 py-3"
            >
              <div className="flex items-center justify-between gap-3">
                <span className="font-mono text-sm font-bold text-text truncate">
                  {c.display_name ?? c.id}
                </span>
                <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
                  {done
                    ? "Ready"
                    : indexing
                      ? `${filesDone.toLocaleString()} / ${filesTotal.toLocaleString()} files${eta != null ? ` · ~${formatEtaBare(eta)}` : ""}`
                      : "Pending…"}
                </span>
              </div>
              {indexing && (
                <Progress
                  className="mt-2"
                  tone="warning"
                  value={pct}
                />
              )}
              {done && (
                <Progress className="mt-2" tone="success" value={100} />
              )}
            </li>
          );
        })}
      </ul>

      <div className="mt-8 flex items-center gap-2 justify-end">
        <Button
          size="lg"
          onClick={onContinue}
          disabled={!anyComplete && watched.length > 0}
        >
          {allComplete
            ? "Continue"
            : anyComplete
              ? "Continue in background"
              : "Waiting for first project…"}
          <ArrowRight className="h-4 w-4" strokeWidth={2} />
        </Button>
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────
// Step 4 — Connect

function StepConnect({ onDone }: { onDone: () => void }) {
  const { status } = useDaemonStatus();
  const corpora = status?.corpora ?? [];
  const activeCorpusId = corpora[0]?.id ?? null;

  return (
    <div>
      <p className="font-mono text-mono-mini font-semibold uppercase tracking-[0.08em] text-accent mb-3">
        Step 4 of 4 · Connect
      </p>
      <h1 className="text-display text-text">
        Hook up your
        <br />
        <span className="text-text-dim">AI tool.</span>
      </h1>
      <p className="font-sans text-base italic text-text-muted mt-4 max-w-xl leading-relaxed">
        ministr is most useful when your AI assistant can ask it questions on
        your behalf. Click Connect on any detected client below to write the
        config file — for CLI clients we'll run a live test, for editors
        you'll need to restart and re-test.
      </p>

      <div className="mt-8">
        <AiAssistantsPanel
          corpora={corpora}
          activeCorpusId={activeCorpusId}
        />
      </div>

      <div className="mt-8 flex items-center gap-2 justify-end">
        <Button size="lg" onClick={onDone}>
          Open ministr
          <ArrowRight className="h-4 w-4" strokeWidth={2} />
        </Button>
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────
// Bits

function PrimaryAction({
  icon: Icon,
  title,
  hint,
  onClick,
  disabled,
  loading,
}: {
  icon: React.ComponentType<{ className?: string; strokeWidth?: number }>;
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
        "group relative flex flex-col items-start gap-2 p-5 text-left cursor-pointer transition-colors duration-150 ease-out",
        "border border-border bg-surface",
        "hover:bg-surface-overlay hover:border-accent",
        "disabled:opacity-50 disabled:cursor-not-allowed",
        "shadow-sm",
      )}
    >
      <div className="flex items-center gap-2">
        {loading ? (
          <Loader2
            className="h-5 w-5 text-accent animate-spin"
            strokeWidth={2}
          />
        ) : (
          <Icon className="h-5 w-5 text-accent" strokeWidth={2} />
        )}
        <span className="font-mono text-base font-bold text-text">
          {title}
        </span>
      </div>
      <p className="font-sans text-sm italic text-text-muted">{hint}</p>
      <ArrowRight
        className="absolute top-4 right-4 h-4 w-4 text-text-dim group-hover:text-accent"
        strokeWidth={2.5}
      />
    </button>
  );
}

function Capability({ title, hint }: { title: string; hint: string }) {
  return (
    <div className="border border-border-soft bg-surface px-3 py-2.5">
      <div className="flex items-center gap-1.5">
        <Sparkles className="h-3 w-3 text-accent" strokeWidth={2.5} />
        <h3 className="font-mono text-mono-mini font-semibold uppercase tracking-[0.08em] text-text">
          {title}
        </h3>
      </div>
      <p className="font-sans text-xs italic text-text-dim mt-1 leading-relaxed">
        {hint}
      </p>
    </div>
  );
}
