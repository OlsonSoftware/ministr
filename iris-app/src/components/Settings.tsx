import { Palette, HardDrive, Cpu } from "lucide-react";
import { Card } from "./ui/card";
import { Button } from "./ui/button";
import type { DaemonStatus } from "../lib/types";

interface SettingsProps {
  status: DaemonStatus;
  theme: string;
  onThemeChange: (theme: "dark" | "light" | "system") => void;
}

export function Settings({ status, theme, onThemeChange }: SettingsProps) {
  return (
    <div className="space-y-3">
      <h2 className="text-sm font-medium text-text-muted uppercase tracking-wider">
        Settings
      </h2>

      <Card>
        <div className="flex items-center gap-2 mb-3">
          <Palette className="h-4 w-4 text-text-muted" />
          <h3 className="font-medium text-sm">Appearance</h3>
        </div>
        <div className="flex gap-2">
          {(["system", "dark", "light"] as const).map((t) => (
            <Button
              key={t}
              variant={theme === t ? "default" : "outline"}
              size="sm"
              onClick={() => onThemeChange(t)}
            >
              {t.charAt(0).toUpperCase() + t.slice(1)}
            </Button>
          ))}
        </div>
      </Card>

      <Card>
        <div className="flex items-center gap-2 mb-3">
          <Cpu className="h-4 w-4 text-text-muted" />
          <h3 className="font-medium text-sm">Embedding Model</h3>
        </div>
        <div className="space-y-2 text-xs">
          <div className="flex justify-between">
            <span className="text-text-muted">Current</span>
            <span className="font-mono">{status.model}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-text-muted">Dimension</span>
            <span>{status.model_dimension}d</span>
          </div>
        </div>
      </Card>

      <Card>
        <div className="flex items-center gap-2 mb-3">
          <HardDrive className="h-4 w-4 text-text-muted" />
          <h3 className="font-medium text-sm">Storage</h3>
        </div>
        <div className="space-y-2 text-xs">
          <div className="flex justify-between">
            <span className="text-text-muted">Memory</span>
            <span>{status.memory_mb.toFixed(0)} MB RSS</span>
          </div>
          <div className="flex justify-between">
            <span className="text-text-muted">Data</span>
            <span className="font-mono text-text-dim">~/.iris/</span>
          </div>
        </div>
      </Card>
    </div>
  );
}
