import { useState, useMemo } from "react";
import {
  Cpu,
  Plus,
  Trash2,
  AlertTriangle,
  ChevronDown,
  Gauge,
  Layers,
} from "lucide-react";
import { Card } from "./ui/card";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Progress } from "./ui/progress";
import { cn } from "../lib/utils";
import {
  pressureFromUtilization,
  PRESSURE_ELEVATED,
  PRESSURE_CRITICAL,
  type Pressure,
} from "../lib/pressure";

const DEFAULT_BUDGET = 200_000;

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
  const pressure = pressureFromUtilization(utilization);

  function addSection(s: MockSection) {
    if (context.find((c) => c.id === s.id)) return;
    setContext((prev) => [...prev, s]);
  }

  function removeSection(id: string) {
    setContext((prev) => prev.filter((c) => c.id !== id));
  }

  function evictRecommendation(): string[] {
    if (utilization < PRESSURE_CRITICAL) return [];
    const sorted = [...context].sort((a, b) => b.tokens - a.tokens);
    const ids: string[] = [];
    let freed = 0;
    const target = tokensUsed - budget * PRESSURE_ELEVATED;
    for (const s of sorted) {
      if (freed >= target) break;
      ids.push(s.id);
      freed += s.tokens;
    }
    return ids;
  }

  const evictIds = evictRecommendation();

  return (
    <div className="space-y-4 ministr-fade-in max-w-3xl">
      <header>
        <h2 className="text-base font-semibold text-text flex items-center gap-2">
          <Cpu className="h-4 w-4 text-accent" />
          Context simulator
        </h2>
        <p className="text-xs text-text-dim mt-0.5">
          Add sections to a mock context window and watch the pressure
          levels + eviction recommendations behave like the real ministr
          budget manager.
        </p>
      </header>

      <Card hover="lift" className="p-4 space-y-3">
        <div className="flex items-center justify-between">
          <span className="text-[11px] font-medium uppercase tracking-wider text-text-dim">
            Token budget
          </span>
          <span className="text-sm font-mono font-semibold tabular-nums text-text">
            {formatTokens(budget)}
          </span>
        </div>
        <input
          type="range"
          min={10000}
          max={1000000}
          step={10000}
          value={budget}
          onChange={(e) => setBudget(Number(e.target.value))}
          className="w-full accent-accent cursor-pointer"
        />
        <div className="flex justify-between text-[10px] text-text-dim font-mono">
          <span>10K</span>
          <span>1M</span>
        </div>
      </Card>

      <Card hover="lift" className="p-4 space-y-3">
        <div className="flex items-center justify-between">
          <span className="flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-wider text-text-dim">
            <Gauge className="h-3 w-3" />
            Context usage
          </span>
          <PressureBadge level={pressure} />
        </div>

        <Progress value={utilization * 100} glow={pressure === "critical" || pressure === "high"} className="h-2" />

        <div className="flex items-center justify-between text-xs">
          <div className="space-x-2">
            <span className="font-mono font-semibold text-text tabular-nums">
              {formatTokens(tokensUsed)}
            </span>
            <span className="text-text-dim">used</span>
          </div>
          <div className="space-x-2">
            <span className="font-mono font-semibold text-text tabular-nums">
              {formatTokens(Math.max(budget - tokensUsed, 0))}
            </span>
            <span className="text-text-dim">remaining</span>
          </div>
        </div>
        <div className="text-[11px] text-text-dim">
          {context.length} section{context.length === 1 ? "" : "s"} loaded ·{" "}
          {(utilization * 100).toFixed(1)}% utilization
        </div>
      </Card>

      {evictIds.length > 0 && (
        <Card className="border-danger/40 bg-danger/5 p-4">
          <div className="flex items-start gap-3">
            <div className="grid h-8 w-8 place-items-center rounded-lg bg-danger/15 text-danger shrink-0">
              <AlertTriangle className="h-4 w-4" />
            </div>
            <div className="flex-1 space-y-2">
              <p className="text-sm font-semibold text-danger">
                Eviction recommended
              </p>
              <p className="text-xs text-text-muted leading-relaxed">
                Remove {evictIds.length} section
                {evictIds.length === 1 ? "" : "s"} to reduce pressure:{" "}
                <span className="text-text">
                  {evictIds
                    .map((id) => context.find((c) => c.id === id)?.name)
                    .filter(Boolean)
                    .join(", ")}
                </span>
              </p>
              <Button
                size="sm"
                variant="danger"
                onClick={() =>
                  setContext((prev) =>
                    prev.filter((c) => !evictIds.includes(c.id)),
                  )
                }
              >
                <Trash2 className="h-3 w-3" />
                Auto-evict recommended
              </Button>
            </div>
          </div>
        </Card>
      )}

      <div className="grid md:grid-cols-2 gap-3">
        <Card className="p-4">
          <button
            onClick={() => setShowSections(!showSections)}
            className="flex items-center gap-1.5 text-xs font-semibold text-text-muted mb-3 cursor-pointer"
          >
            <ChevronDown
              className={cn(
                "h-3 w-3 transition-transform",
                !showSections && "-rotate-90",
              )}
            />
            Available sections
            <Badge variant="muted" className="ml-1">
              {SAMPLE_SECTIONS.length - context.length}
            </Badge>
          </button>
          {showSections && (
            <div className="space-y-1 max-h-72 overflow-y-auto -mr-1 pr-1">
              {SAMPLE_SECTIONS.map((s) => {
                const inContext = context.find((c) => c.id === s.id);
                return (
                  <button
                    key={s.id}
                    onClick={() => !inContext && addSection(s)}
                    disabled={!!inContext}
                    className={cn(
                      "w-full text-left flex items-center justify-between gap-2 rounded-lg px-2.5 py-2 text-xs transition-all duration-120 border cursor-pointer",
                      inContext
                        ? "border-transparent opacity-40 cursor-not-allowed"
                        : "border-border/50 bg-surface-raised/40 hover:border-[var(--color-accent-ring)] hover:bg-[var(--color-accent-soft)]",
                    )}
                  >
                    <span className="truncate text-text">{s.name}</span>
                    <div className="flex items-center gap-2 shrink-0">
                      <span className="text-[10px] font-mono tabular-nums text-text-dim">
                        {s.tokens}
                      </span>
                      {!inContext && (
                        <Plus className="h-3 w-3 text-accent" />
                      )}
                    </div>
                  </button>
                );
              })}
            </div>
          )}
        </Card>

        <Card className="p-4">
          <h3 className="flex items-center gap-1.5 text-xs font-semibold text-text-muted mb-3">
            <Layers className="h-3 w-3" />
            Context window
            <Badge variant="default" className="ml-1">
              {context.length}
            </Badge>
          </h3>
          {context.length === 0 ? (
            <div className="flex flex-col items-center justify-center gap-2 py-8 text-center">
              <p className="text-sm font-medium text-text">Empty window</p>
              <p className="text-xs text-text-dim max-w-[220px]">
                Add sections from the left panel to see pressure rise.
              </p>
            </div>
          ) : (
            <div className="space-y-1 max-h-72 overflow-y-auto -mr-1 pr-1">
              {context.map((s) => (
                <div
                  key={s.id}
                  className={cn(
                    "flex items-center justify-between gap-2 rounded-lg px-2.5 py-2 text-xs border",
                    evictIds.includes(s.id)
                      ? "border-danger/40 bg-danger/5"
                      : "border-border/50 bg-surface-raised/40",
                  )}
                >
                  <span className="truncate text-text">{s.name}</span>
                  <div className="flex items-center gap-2 shrink-0">
                    <span className="text-[10px] font-mono tabular-nums text-text-dim">
                      {s.tokens}
                    </span>
                    <button
                      onClick={() => removeSection(s.id)}
                      className="text-text-dim hover:text-danger cursor-pointer"
                      title="Remove"
                    >
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </Card>
      </div>
    </div>
  );
}

function PressureBadge({ level }: { level: Pressure }) {
  const variant: Record<
    Pressure,
    "success" | "default" | "warning" | "danger" | "muted"
  > = {
    none: "muted",
    low: "success",
    medium: "default",
    high: "warning",
    critical: "danger",
  };
  const dot = level === "high" || level === "critical";
  return (
    <Badge variant={variant[level]} dot={dot}>
      {level}
    </Badge>
  );
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toString();
}
