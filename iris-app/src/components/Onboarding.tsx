import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Zap, FolderOpen, ArrowRight, ArrowLeft, CheckCircle2, Search } from "lucide-react";
import { Button } from "./ui/button";
import { Card } from "./ui/card";
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
    if (step === "detect") {
      scanProjects();
    }
  }, [step]);

  async function scanProjects() {
    setScanning(true);
    try {
      const projects = await invoke<DetectedProject[]>("detect_projects");
      setDetected(projects);
      // Pre-select all detected projects
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
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  }

  function toggleAll() {
    if (selected.size === detected.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(detected.map((p) => p.path)));
    }
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
    <div className="flex items-center justify-center h-full p-8">
      {step === "welcome" && (
        <Card className="max-w-md w-full text-center space-y-6 py-8">
          <div className="flex justify-center">
            <div className="rounded-2xl bg-accent/10 p-4">
              <Zap className="h-10 w-10 text-accent" />
            </div>
          </div>

          <div>
            <h1 className="text-xl font-semibold mb-2">Welcome to iris</h1>
            <p className="text-sm text-text-muted leading-relaxed">
              iris manages LLM agent context like a CPU cache controller --
              with session tracking, predictive prefetching, budget management,
              and coherence protocols.
            </p>
          </div>

          <div className="text-left space-y-2 px-4">
            <Feature text="Semantic search across docs and code" />
            <Feature text="Token budget management and deduplication" />
            <Feature text="Predictive prefetching for faster responses" />
            <Feature text="Session-aware context coherence" />
          </div>

          <div className="space-y-3 pt-2">
            <Button className="w-full" onClick={() => setStep("detect")}>
              <Search className="mr-2 h-4 w-4" />
              Scan for Projects
            </Button>

            <Button variant="ghost" className="w-full" onClick={addManually}>
              <FolderOpen className="mr-2 h-4 w-4" />
              Add Project Manually
            </Button>

            <button
              onClick={dismiss}
              className="text-xs text-text-dim hover:text-text-muted cursor-pointer flex items-center justify-center gap-1 mx-auto"
            >
              Skip for now
              <ArrowRight className="h-3 w-3" />
            </button>
          </div>
        </Card>
      )}

      {step === "detect" && (
        <Card className="max-w-lg w-full space-y-4 py-6">
          <div className="text-center">
            <h2 className="text-lg font-semibold mb-1">Detected Projects</h2>
            <p className="text-xs text-text-dim">
              Scanning ~/Code, ~/Projects, ~/Developer, ~/src for .iris.toml files
            </p>
          </div>

          {scanning ? (
            <div className="flex items-center justify-center py-8">
              <div className="animate-spin rounded-full h-6 w-6 border-2 border-accent border-t-transparent" />
              <span className="ml-2 text-sm text-text-muted">Scanning...</span>
            </div>
          ) : detected.length === 0 ? (
            <div className="text-center py-6">
              <p className="text-sm text-text-muted mb-3">
                No projects with .iris.toml found.
              </p>
              <p className="text-xs text-text-dim">
                Add a .iris.toml to your project root to enable auto-detection,
                or add a project manually.
              </p>
            </div>
          ) : (
            <>
              <div className="flex items-center justify-between px-1">
                <button
                  onClick={toggleAll}
                  className="text-xs text-accent hover:text-accent/80 cursor-pointer"
                >
                  {selected.size === detected.length ? "Deselect all" : "Select all"}
                </button>
                <span className="text-xs text-text-dim">
                  {selected.size} of {detected.length} selected
                </span>
              </div>

              <div className="max-h-64 overflow-y-auto space-y-1">
                {detected.map((project) => (
                  <label
                    key={project.path}
                    className={`flex items-center gap-3 px-3 py-2 rounded-lg cursor-pointer transition-colors ${
                      selected.has(project.path)
                        ? "bg-accent/5 border border-accent/20"
                        : "hover:bg-surface-hover border border-transparent"
                    }`}
                  >
                    <input
                      type="checkbox"
                      checked={selected.has(project.path)}
                      onChange={() => toggleProject(project.path)}
                      className="rounded border-border-default text-accent focus:ring-accent"
                    />
                    <div className="min-w-0 flex-1">
                      <div className="text-sm font-medium truncate">{project.name}</div>
                      <div className="text-xs text-text-dim font-mono truncate">
                        {project.path}
                      </div>
                    </div>
                  </label>
                ))}
              </div>
            </>
          )}

          <div className="flex items-center gap-2 pt-2">
            <Button variant="ghost" size="sm" onClick={() => setStep("welcome")}>
              <ArrowLeft className="mr-1 h-3.5 w-3.5" />
              Back
            </Button>
            <div className="flex-1" />
            <Button variant="ghost" size="sm" onClick={addManually}>
              <FolderOpen className="mr-1 h-3.5 w-3.5" />
              Add Manually
            </Button>
            <Button
              size="sm"
              onClick={importSelected}
              disabled={importing}
            >
              {importing
                ? "Importing..."
                : selected.size > 0
                  ? `Add ${selected.size} Project${selected.size !== 1 ? "s" : ""}`
                  : "Skip"}
            </Button>
          </div>
        </Card>
      )}

      {step === "done" && (
        <Card className="max-w-md w-full text-center space-y-6 py-8">
          <div className="flex justify-center">
            <div className="rounded-2xl bg-green-500/10 p-4">
              <CheckCircle2 className="h-10 w-10 text-green-500" />
            </div>
          </div>

          <div>
            <h2 className="text-lg font-semibold mb-2">All Set!</h2>
            <p className="text-sm text-text-muted">
              {importedCount === 1
                ? "1 project is being indexed."
                : `${importedCount} projects are being indexed.`}
              {" "}You can add more projects anytime from the dashboard.
            </p>
          </div>

          <Button className="w-full" onClick={dismiss}>
            Open Dashboard
            <ArrowRight className="ml-2 h-4 w-4" />
          </Button>
        </Card>
      )}
    </div>
  );
}

function Feature({ text }: { text: string }) {
  return (
    <div className="flex items-center gap-2 text-sm text-text-muted">
      <div className="h-1.5 w-1.5 rounded-full bg-accent shrink-0" />
      {text}
    </div>
  );
}
