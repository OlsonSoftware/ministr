import { invoke } from "@tauri-apps/api/core";
import { Zap, FolderOpen, ArrowRight } from "lucide-react";
import { Button } from "./ui/button";
import { Card } from "./ui/card";

interface OnboardingProps {
  onDismiss: () => void;
}

export function Onboarding({ onDismiss }: OnboardingProps) {
  async function addProject() {
    await invoke("add_project_dialog");
    await dismiss();
  }

  async function dismiss() {
    await invoke("dismiss_onboarding");
    onDismiss();
  }

  return (
    <div className="flex items-center justify-center h-full p-8">
      <Card className="max-w-md w-full text-center space-y-6 py-8">
        <div className="flex justify-center">
          <div className="rounded-2xl bg-accent/10 p-4">
            <Zap className="h-10 w-10 text-accent" />
          </div>
        </div>

        <div>
          <h1 className="text-xl font-semibold mb-2">Welcome to iris</h1>
          <p className="text-sm text-text-muted leading-relaxed">
            iris manages LLM agent context like a CPU cache controller.
            Add a project to start indexing your codebase.
          </p>
        </div>

        <div className="space-y-3">
          <Button className="w-full" onClick={addProject}>
            <FolderOpen className="mr-2 h-4 w-4" />
            Add Your First Project
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
    </div>
  );
}
