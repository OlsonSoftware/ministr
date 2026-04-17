import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  GitBranch,
  Search,
  Code2,
  ArrowRight,
  Loader2,
} from "lucide-react";
import { Card } from "./ui/card";
import { Button } from "./ui/button";
import { Badge } from "./ui/badge";
import { cn } from "../lib/utils";
import type { DaemonStatus, SymbolInfo, SymbolRef } from "../lib/types";

interface Props {
  status: DaemonStatus;
}

interface GraphNode {
  id: string;
  name: string;
  kind: string;
  x: number;
  y: number;
}

interface GraphEdge {
  from: string;
  to: string;
  kind: string;
}

export function SymbolGraph({ status }: Props) {
  const [corpusId, setCorpusId] = useState(status.corpora[0]?.id ?? "");
  const [query, setQuery] = useState("");
  const [symbols, setSymbols] = useState<SymbolInfo[]>([]);
  const [selected, setSelected] = useState<SymbolInfo | null>(null);
  const [refs, setRefs] = useState<SymbolRef[]>([]);
  const [loading, setLoading] = useState(false);

  async function searchSymbols() {
    if (!query.trim() || !corpusId) return;
    setLoading(true);
    try {
      const r = await invoke<SymbolInfo[]>("search_symbols", {
        corpusId,
        query: query.trim(),
        kind: null,
      });
      setSymbols(r);
      setSelected(null);
      setRefs([]);
    } catch {
      /* ignore */
    } finally {
      setLoading(false);
    }
  }

  async function selectSymbol(sym: SymbolInfo) {
    setSelected(sym);
    try {
      const r = await invoke<SymbolRef[]>("symbol_references", {
        corpusId,
        symbolId: sym.id,
      });
      setRefs(r);
    } catch {
      setRefs([]);
    }
  }

  const nodes: GraphNode[] = [];
  const edges: GraphEdge[] = [];
  if (selected) {
    const nodeSet = new Map<string, GraphNode>();
    nodeSet.set(selected.name, {
      id: selected.id,
      name: selected.name,
      kind: selected.kind,
      x: 0,
      y: 0,
    });

    for (const r of refs) {
      if (!nodeSet.has(r.from_name)) {
        nodeSet.set(r.from_name, {
          id: r.from_name,
          name: r.from_name,
          kind: "ref",
          x: 0,
          y: 0,
        });
      }
      if (!nodeSet.has(r.to_name)) {
        nodeSet.set(r.to_name, {
          id: r.to_name,
          name: r.to_name,
          kind: "ref",
          x: 0,
          y: 0,
        });
      }
      edges.push({ from: r.from_name, to: r.to_name, kind: r.ref_kind });
    }

    const allNodes = Array.from(nodeSet.values());
    const cx = 250;
    const cy = 175;
    const radius = Math.min(140, allNodes.length * 22);
    allNodes.forEach((n, i) => {
      if (n.name === selected.name) {
        n.x = cx;
        n.y = cy;
      } else {
        const angle = (2 * Math.PI * i) / Math.max(allNodes.length - 1, 1);
        n.x = cx + radius * Math.cos(angle);
        n.y = cy + radius * Math.sin(angle);
      }
    });
    nodes.push(...allNodes);
  }

  return (
    <div className="space-y-4 iris-fade-in">
      <header className="flex items-end justify-between gap-4 flex-wrap">
        <div>
          <h2 className="text-base font-semibold text-text flex items-center gap-2">
            <GitBranch className="h-4 w-4 text-accent" />
            Symbol graph
          </h2>
          <p className="text-xs text-text-dim mt-0.5">
            Search symbols, then pick one to see its call/implementation graph.
          </p>
        </div>
        <select
          value={corpusId}
          onChange={(e) => setCorpusId(e.target.value)}
          className="h-8 rounded-md border border-border/70 bg-surface-raised px-2.5 text-xs font-mono text-text cursor-pointer focus:outline-none focus:border-[var(--color-accent-ring)] focus:shadow-[0_0_0_3px_var(--color-accent-soft)]"
        >
          {status.corpora.map((c) => (
            <option key={c.id} value={c.id}>
              {c.id}
            </option>
          ))}
        </select>
      </header>

      <Card className="p-3">
        <form
          onSubmit={(e) => {
            e.preventDefault();
            searchSymbols();
          }}
          className="flex gap-2"
        >
          <div className="relative flex-1">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-text-dim" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search symbols by name…"
              className="h-9 w-full rounded-md border border-border/70 bg-surface-raised pl-9 pr-3 text-sm text-text placeholder:text-text-dim font-mono focus:outline-none focus:border-[var(--color-accent-ring)] focus:shadow-[0_0_0_3px_var(--color-accent-soft)]"
            />
          </div>
          <Button type="submit" disabled={loading || !query.trim()}>
            {loading ? <Loader2 className="h-3.5 w-3.5 iris-spin" /> : <Search className="h-3.5 w-3.5" />}
            Search
          </Button>
        </form>
      </Card>

      <div className="grid grid-cols-1 md:grid-cols-[260px_1fr] gap-3 min-h-[340px]">
        <Card hover="lift" className="p-0 overflow-hidden flex flex-col">
          <div className="px-3 py-2 border-b border-border/60">
            <h3 className="text-[11px] font-semibold uppercase tracking-wider text-text-dim">
              Symbols
              {symbols.length > 0 && (
                <span className="ml-2 text-text-muted font-mono normal-case tabular-nums">
                  {symbols.length}
                </span>
              )}
            </h3>
          </div>
          <div className="flex-1 overflow-y-auto p-1 max-h-[420px]">
            {symbols.length === 0 ? (
              <div className="flex flex-col items-center justify-center gap-1 py-10 text-center">
                <p className="text-xs text-text-dim">
                  {query ? "No symbols found" : "Search to begin"}
                </p>
              </div>
            ) : (
              symbols.map((s) => {
                const isSelected = selected?.id === s.id;
                return (
                  <button
                    key={s.id}
                    onClick={() => selectSymbol(s)}
                    className={cn(
                      "w-full text-left flex flex-col rounded-md px-2.5 py-2 text-xs transition-all duration-120 cursor-pointer",
                      isSelected
                        ? "bg-[var(--color-accent-soft)] shadow-[inset_0_0_0_1px_var(--color-accent-ring)]"
                        : "hover:bg-surface-overlay/50",
                    )}
                  >
                    <div className="flex items-center gap-1.5">
                      <Code2
                        className={cn(
                          "h-3 w-3 shrink-0",
                          isSelected ? "text-accent" : "text-text-dim",
                        )}
                      />
                      <span
                        className={cn(
                          "font-mono font-semibold truncate",
                          isSelected ? "text-accent" : "text-text",
                        )}
                      >
                        {s.name}
                      </span>
                    </div>
                    <span className="mt-0.5 text-[10px] font-mono uppercase tracking-wider text-text-dim">
                      {s.kind}
                    </span>
                  </button>
                );
              })
            )}
          </div>
        </Card>

        <Card hover="lift" className="p-0 overflow-hidden">
          {selected ? (
            <svg viewBox="0 0 500 350" className="w-full h-full min-h-[340px]">
              {/* soft grid */}
              <defs>
                <pattern
                  id="sg-grid"
                  width="24"
                  height="24"
                  patternUnits="userSpaceOnUse"
                >
                  <path
                    d="M 24 0 L 0 0 0 24"
                    fill="none"
                    stroke="currentColor"
                    strokeOpacity="0.05"
                    strokeWidth="0.5"
                  />
                </pattern>
              </defs>
              <rect width="100%" height="100%" fill="url(#sg-grid)" />

              {edges.map((e, i) => {
                const from = nodes.find((n) => n.name === e.from);
                const to = nodes.find((n) => n.name === e.to);
                if (!from || !to) return null;
                return (
                  <line
                    key={i}
                    x1={from.x}
                    y1={from.y}
                    x2={to.x}
                    y2={to.y}
                    stroke="rgb(129 140 248)"
                    strokeOpacity={0.5}
                    strokeWidth={1}
                  />
                );
              })}

              {nodes.map((n) => {
                const isCenter = n.name === selected.name;
                return (
                  <g key={n.name}>
                    {isCenter && (
                      <circle
                        cx={n.x}
                        cy={n.y}
                        r={14}
                        fill="rgb(129 140 248)"
                        opacity={0.15}
                      />
                    )}
                    <circle
                      cx={n.x}
                      cy={n.y}
                      r={isCenter ? 8 : 5}
                      fill={isCenter ? "rgb(129 140 248)" : "rgb(100 116 139)"}
                      opacity={isCenter ? 1 : 0.7}
                    />
                    <text
                      x={n.x}
                      y={n.y + (isCenter ? 22 : 17)}
                      textAnchor="middle"
                      fill={isCenter ? "rgb(229 231 235)" : "rgb(156 163 175)"}
                      fontSize={isCenter ? 10 : 9}
                      fontFamily="var(--font-mono)"
                      fontWeight={isCenter ? 600 : 500}
                    >
                      {n.name.length > 22 ? n.name.slice(0, 22) + "…" : n.name}
                    </text>
                  </g>
                );
              })}
            </svg>
          ) : (
            <div className="flex flex-col items-center justify-center gap-2 h-full py-10 text-center">
              <div className="grid h-12 w-12 place-items-center rounded-xl bg-surface-overlay text-text-dim">
                <GitBranch className="h-5 w-5" />
              </div>
              <p className="text-sm font-medium text-text">Pick a symbol</p>
              <p className="max-w-xs text-xs text-text-dim">
                Select a symbol from the list to render its reference graph.
              </p>
            </div>
          )}
        </Card>
      </div>

      {refs.length > 0 && (
        <Card hover="lift" className="p-0 overflow-hidden">
          <div className="px-4 py-2.5 border-b border-border/60">
            <h3 className="text-[11px] font-semibold uppercase tracking-wider text-text-dim">
              References
              <span className="ml-2 font-mono text-text-muted tabular-nums normal-case">
                {refs.length}
              </span>
            </h3>
          </div>
          <div className="max-h-64 overflow-y-auto divide-y divide-border/40">
            {refs.map((r, i) => (
              <div
                key={i}
                className="flex items-center gap-2 px-4 py-2 text-xs hover:bg-surface-overlay/50"
              >
                <span className="font-mono text-text truncate">
                  {r.from_name}
                </span>
                <ArrowRight className="h-3 w-3 text-accent shrink-0" />
                <span className="font-mono text-text truncate">
                  {r.to_name}
                </span>
                <Badge variant="muted" className="ml-auto shrink-0">
                  {r.ref_kind}
                </Badge>
              </div>
            ))}
          </div>
        </Card>
      )}
    </div>
  );
}
