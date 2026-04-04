import { useState, useMemo } from "react";
import { Cpu, Plus, Trash2, AlertTriangle, ChevronDown } from "lucide-react";
import { Card } from "./ui/card";

const DEFAULT_BUDGET = 200_000;
const WARN_THRESHOLD = 0.7;
const CRITICAL_THRESHOLD = 0.9;

interface MockSection {
  id: string;
  name: string;
  tokens: number;
}

const SAMPLE_SECTIONS: MockSection[] = [
  { id: "s1", name: "Module overview (README)", tokens: 800 },
  { id: "s2", name: "Config struct definition", tokens: 350 },
  { id: "s3", name: "Main entry point", tokens: 1200 },
  { id: "s4", name: "Error handling module", tokens: 950 },
  { id: "s5", name: "Database migration", tokens: 2100 },
  { id: "s6", name: "API routes", tokens: 1800 },
  { id: "s7", name: "Auth middleware", tokens: 600 },
  { id: "s8", name: "Test helpers", tokens: 1500 },
  { id: "s9", name: "Type definitions", tokens: 400 },
  { id: "s10", name: "CLI argument parser", tokens: 750 },
  { id: "s11", name: "Logging setup", tokens: 300 },
  { id: "s12", name: "Build script", tokens: 500 },
];

export function ContextSimulator() {
  const [budget, setBudget] = useState(DEFAULT_BUDGET);
  const [context, setContext] = useState<MockSection[]>([]);
  const [showSections, setShowSections] = useState(true);

  const tokensUsed = useMemo(
    () => context.reduce((s, c) => s + c.tokens, 0),
    [context],
  );
  const utilization = budget > 0 ? tokensUsed / budget : 0;
  const pressure = getPressure(utilization);

  function addSection(s: MockSection) {
    if (context.find((c) => c.id === s.id)) return;
    setContext((prev) => [...prev, s]);
  }

  function removeSection(id: string) {
    setContext((prev) => prev.filter((c) => c.id !== id));
  }

  function evictRecommendation(): string[] {
    if (utilization < CRITICAL_THRESHOLD) return [];
    // Suggest evicting largest sections first
    const sorted = [...context].sort((a, b) => b.tokens - a.tokens);
    const ids: string[] = [];
    let freed = 0;
    const target = tokensUsed - budget * WARN_THRESHOLD;
    for (const s of sorted) {
      if (freed >= target) break;
      ids.push(s.id);
      freed += s.tokens;
    }
    return ids;
  }

  const evictIds = evictRecommendation();

  return (
    <div className="space-y-3">
      <h2 className="text-sm font-medium text-text-muted uppercase tracking-wider flex items-center gap-2">
        <Cpu className="h-4 w-4" /> Context Window Simulator
      </h2>

      {/* Budget slider */}
      <Card>
        <div className="flex items-center justify-between mb-2">
          <span className="text-xs text-text-dim">Token Budget</span>
          <span className="text-sm font-mono">{formatTokens(budget)}</span>
        </div>
        <input
          type="range"
          min={10000}
          max={1000000}
          step={10000}
          value={budget}
          onChange={(e) => setBudget(Number(e.target.value))}
          className="w-full accent-accent"
        />
        <div className="flex justify-between text-xs text-text-dim mt-1">
          <span>10K</span>
          <span>1M</span>
        </div>
      </Card>

      {/* Status */}
      <Card>
        <div className="flex items-center justify-between mb-2">
          <span className="text-xs text-text-dim">Context Usage</span>
          <PressureBadge level={pressure} />
        </div>

        <div className="h-3 rounded-full bg-surface-overlay overflow-hidden mb-2">
          <div
            className={`h-full rounded-full transition-all ${
              pressure === "critical"
                ? "bg-danger"
                : pressure === "high"
                  ? "bg-warning"
                  : pressure === "medium"
                    ? "bg-accent"
                    : "bg-green-500"
            }`}
            style={{ width: `${Math.min(utilization * 100, 100)}%` }}
          />
        </div>

        <div className="flex justify-between text-xs text-text-dim">
          <span>{formatTokens(tokensUsed)} used</span>
          <span>{formatTokens(Math.max(budget - tokensUsed, 0))} remaining</span>
        </div>
        <div className="text-xs text-text-dim mt-1">
          {context.length} sections loaded · {(utilization * 100).toFixed(1)}% utilization
        </div>
      </Card>

      {/* Eviction warning */}
      {evictIds.length > 0 && (
        <div className="flex items-start gap-2 bg-danger/5 border border-danger/30 rounded-lg p-3">
          <AlertTriangle className="h-4 w-4 text-danger shrink-0 mt-0.5" />
          <div>
            <p className="text-xs text-danger font-medium">Eviction recommended</p>
            <p className="text-xs text-text-dim mt-0.5">
              Remove {evictIds.length} section(s) to reduce pressure:{" "}
              {evictIds
                .map((id) => context.find((c) => c.id === id)?.name)
                .join(", ")}
            </p>
            <button
              onClick={() => setContext((prev) => prev.filter((c) => !evictIds.includes(c.id)))}
              className="mt-1 text-xs text-danger hover:text-danger/80 underline cursor-pointer"
            >
              Auto-evict recommended sections
            </button>
          </div>
        </div>
      )}

      <div className="grid md:grid-cols-2 gap-3">
        {/* Available sections */}
        <div>
          <button
            onClick={() => setShowSections(!showSections)}
            className="flex items-center gap-1 text-xs font-medium text-text-muted mb-2 cursor-pointer"
          >
            <ChevronDown className={`h-3 w-3 transition-transform ${showSections ? "" : "-rotate-90"}`} />
            Available Sections
          </button>
          {showSections && (
            <div className="space-y-1 max-h-64 overflow-y-auto">
              {SAMPLE_SECTIONS.map((s) => {
                const inContext = context.find((c) => c.id === s.id);
                return (
                  <button
                    key={s.id}
                    onClick={() => !inContext && addSection(s)}
                    disabled={!!inContext}
                    className={`w-full text-left flex items-center justify-between px-2 py-1.5 rounded text-xs transition-colors cursor-pointer ${
                      inContext
                        ? "opacity-40 cursor-not-allowed"
                        : "hover:bg-surface-overlay"
                    }`}
                  >
                    <span className="truncate">{s.name}</span>
                    <div className="flex items-center gap-2 shrink-0">
                      <span className="text-text-dim">{s.tokens} tok</span>
                      {!inContext && <Plus className="h-3 w-3 text-accent" />}
                    </div>
                  </button>
                );
              })}
            </div>
          )}
        </div>

        {/* Current context */}
        <div>
          <h3 className="text-xs font-medium text-text-muted mb-2">
            Context Window ({context.length})
          </h3>
          {context.length === 0 ? (
            <p className="text-xs text-text-dim">Add sections from the left panel.</p>
          ) : (
            <div className="space-y-1 max-h-64 overflow-y-auto">
              {context.map((s) => (
                <div
                  key={s.id}
                  className={`flex items-center justify-between px-2 py-1.5 rounded text-xs ${
                    evictIds.includes(s.id)
                      ? "bg-danger/5 border border-danger/30"
                      : "bg-surface-raised"
                  }`}
                >
                  <span className="truncate">{s.name}</span>
                  <div className="flex items-center gap-2 shrink-0">
                    <span className="text-text-dim">{s.tokens} tok</span>
                    <button
                      onClick={() => removeSection(s.id)}
                      className="text-text-dim hover:text-danger cursor-pointer"
                    >
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function PressureBadge({ level }: { level: string }) {
  const colors: Record<string, string> = {
    none: "bg-green-500/10 text-green-500",
    low: "bg-green-500/10 text-green-500",
    medium: "bg-accent/10 text-accent",
    high: "bg-warning/10 text-warning",
    critical: "bg-danger/10 text-danger",
  };
  return (
    <span className={`text-xs px-2 py-0.5 rounded-full ${colors[level] ?? colors.low}`}>
      {level}
    </span>
  );
}

function getPressure(utilization: number): string {
  if (utilization >= CRITICAL_THRESHOLD) return "critical";
  if (utilization >= WARN_THRESHOLD) return "high";
  if (utilization >= 0.4) return "medium";
  if (utilization > 0) return "low";
  return "none";
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toString();
}
