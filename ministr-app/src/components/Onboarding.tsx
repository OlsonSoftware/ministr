import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowRight,
  Check,
  FolderOpen,
  Loader2,
  Search,
  Sparkles,
} from "lucide-react";
import { cn } from "../lib/utils";
import type { DetectedProject } from "../lib/types";

interface OnboardingProps {
  onDismiss: () => void;
}

/**
 * Single-screen hero canvas.
 *
 * Replaces the previous 3-step wizard (welcome → detect → done) with one
 * goal-oriented surface. The user sees the offer immediately ("ask your
 * codebase questions"), picks a folder or auto-detects, and the imported
 * projects pile up inline before they dismiss into the workspace.
 *
 * No back/next, no card framing, no step dots. The display-type headline
 * is the visual anchor; everything else is one click away.
 */
export function Onboarding({ onDismiss }: OnboardingProps) {
  const [detected, setDetected] = useState<DetectedProject[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [scanning, setScanning] = useState(false);
  const [showScanning, setShowScanning] = useState(false);
  const [importing, setImporting] = useState(false);
  const [importedCount, setImportedCount] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const scanTimer = useRef<number | null>(null);

  // Debounce the scanning indicator so flash-fast scans don't blink at the user.
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
      setImportedCount((c) => c + ids.length);
      setDetected(null);
      setSelected(new Set());
    } catch (err) {
      console.error("[ministr] register_projects_batch error:", err);
      setError(String(err));
    } finally {
      setImporting(false);
    }
  }

  async function pickFolder() {
    setError(null);
    try {
      await invoke("add_project_dialog");
      setImportedCount((c) => c + 1);
    } catch (err) {
      console.error("[ministr] add_project_dialog error:", err);
      setError(String(err));
    }
  }

  async function dismiss() {
    await invoke("dismiss_onboarding");
    onDismiss();
  }

  return (
    <div className="flex h-full flex-col bg-bg text-text">
      {/* Top bar — wordmark + skip link. Minimal chrome on purpose. */}
      <header className="flex items-center justify-between gap-4 px-8 py-4 shrink-0">
        <span className="ministr-wordmark">ministr</span>
        <button
          onClick={dismiss}
          className={cn(
            "inline-flex items-center gap-1.5 cursor-pointer transition-none rounded-sm",
            "border border-border-soft bg-surface px-3 py-1",
            "font-mono text-mono-mini font-semibold uppercase tracking-[0.05em]",
            "text-text-dim hover:text-text hover:border-border",
          )}
        >
          {importedCount > 0 ? "Continue to workspace" : "Skip for now"}
          <ArrowRight className="h-3 w-3" strokeWidth={2.5} />
        </button>
      </header>

      <main className="flex-1 min-h-0 overflow-y-auto">
        <div className="mx-auto max-w-3xl px-8 py-6">
          {/* Hero headline — the display type anchor. */}
          <div className="mb-8">
            <p className="font-mono text-mono-mini font-semibold uppercase tracking-[0.05em] text-accent mb-3">
              Welcome
            </p>
            <h1 className="text-display text-text">
              Ask your codebase
              <br />
              <span className="text-text-dim">anything.</span>
            </h1>
            <p className="font-serif text-base italic text-text-muted mt-4 max-w-xl leading-relaxed">
              Pick a folder. ministr indexes it locally — code, docs,
              symbols, cross-language bridges — then answers questions
              about it with cited source.
            </p>
          </div>

          {/* Primary actions — two equal-weight buttons. */}
          {!detected && (
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 mb-6">
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
                hint="Scan common dev dirs (~/code, ~/dev, ~/Documents)."
                onClick={autoDetect}
                disabled={scanning || importing}
                loading={showScanning}
              />
            </div>
          )}

          {/* Imported chip — feedback when the user adds a project via picker. */}
          {importedCount > 0 && !detected && (
            <div
              className={cn(
                "border-2 border-accent bg-surface-overlay px-4 py-3 mb-4",
                "ministr-pin-in",
              )}
            >
              <div className="flex items-center gap-2">
                <Check className="h-4 w-4 text-accent" strokeWidth={2.5} />
                <span className="font-mono text-mono-mini font-semibold uppercase tracking-[0.05em] text-text">
                  Indexed {importedCount} project
                  {importedCount === 1 ? "" : "s"}
                </span>
                <span className="flex-1" />
                <button
                  onClick={dismiss}
                  className={cn(
                    "inline-flex items-center gap-1 px-2 py-0.5 cursor-pointer transition-none rounded-sm",
                    "border border-accent bg-accent text-accent-fg-on",
                    "font-mono text-mono-mini font-semibold uppercase tracking-[0.05em]",
                    "hover:opacity-90",
                  )}
                >
                  Open workspace
                  <ArrowRight className="h-3 w-3" strokeWidth={2.5} />
                </button>
              </div>
            </div>
          )}

          {/* Detected list — appears inline below the actions when scan runs. */}
          {detected && (
            <div className="border-2 border-border bg-surface">
              <header className="flex items-center justify-between gap-2 border-b-2 border-border bg-surface-overlay px-4 py-2">
                <h2 className="font-mono text-mono-mini font-semibold uppercase tracking-[0.05em] text-text">
                  {detected.length === 0
                    ? "No projects detected"
                    : `Detected ${detected.length} project${detected.length === 1 ? "" : "s"}`}
                </h2>
                {detected.length > 0 && (
                  <button
                    onClick={toggleAll}
                    className={cn(
                      "font-mono text-mono-mini font-semibold uppercase tracking-[0.05em]",
                      "text-text-dim hover:text-text cursor-pointer transition-none",
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
                  <p className="font-serif text-sm italic text-text-dim mb-3">
                    Nothing in the usual places. Try Pick a folder.
                  </p>
                  <button
                    onClick={() => setDetected(null)}
                    className={cn(
                      "inline-flex items-center gap-1 px-2 py-0.5 cursor-pointer transition-none rounded-sm",
                      "border border-border-soft bg-surface text-text-muted",
                      "font-mono text-mono-mini font-semibold uppercase tracking-[0.05em]",
                      "hover:text-text hover:border-border",
                    )}
                  >
                    Back
                  </button>
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
                              "flex items-start gap-3 px-4 py-2.5 cursor-pointer transition-none",
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
                      <button
                        onClick={() => setDetected(null)}
                        disabled={importing}
                        className={cn(
                          "inline-flex items-center gap-1 px-2 py-1 cursor-pointer transition-none rounded-sm",
                          "border border-border-soft bg-surface text-text-muted",
                          "font-mono text-mono-mini font-semibold uppercase tracking-[0.05em]",
                          "hover:text-text hover:border-border",
                        )}
                      >
                        Back
                      </button>
                      <button
                        onClick={importSelected}
                        disabled={importing || selected.size === 0}
                        className={cn(
                          "inline-flex items-center gap-1.5 px-3 py-1 cursor-pointer transition-none rounded-sm",
                          "border-2 border-accent bg-accent text-accent-fg-on",
                          "font-mono text-mono-mini font-semibold uppercase tracking-[0.05em]",
                          "hover:opacity-90 disabled:opacity-50 disabled:cursor-not-allowed",
                        )}
                      >
                        {importing && (
                          <Loader2 className="h-3 w-3 animate-spin" />
                        )}
                        Index {selected.size} project
                        {selected.size === 1 ? "" : "s"}
                      </button>
                    </div>
                  </footer>
                </>
              )}
            </div>
          )}

          {error && (
            <p className="mt-3 font-mono text-mono-mini text-danger">
              {error}
            </p>
          )}

          {/* Capability strip — three brutalist tiles below the action area. */}
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
      </main>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
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
        "group relative flex flex-col items-start gap-2 p-5 text-left cursor-pointer transition-none",
        "border-2 border-border bg-surface",
        "hover:bg-surface-overlay hover:border-accent",
        "disabled:opacity-50 disabled:cursor-not-allowed",
        "shadow-sm",
      )}
    >
      <div className="flex items-center gap-2">
        {loading ? (
          <Loader2 className="h-5 w-5 text-accent animate-spin" strokeWidth={2} />
        ) : (
          <Icon className="h-5 w-5 text-accent" strokeWidth={2} />
        )}
        <span className="font-mono text-base font-bold text-text">{title}</span>
      </div>
      <p className="font-serif text-sm italic text-text-muted">{hint}</p>
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
        <h3 className="font-mono text-mono-mini font-semibold uppercase tracking-[0.05em] text-text">
          {title}
        </h3>
      </div>
      <p className="font-serif text-xs italic text-text-dim mt-1 leading-relaxed">
        {hint}
      </p>
    </div>
  );
}
