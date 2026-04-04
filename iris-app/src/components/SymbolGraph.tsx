import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { GitBranch, Search, Code2 } from "lucide-react";
import { Card } from "./ui/card";
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

  // Build a mini graph from the selected symbol + its refs
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

    // Simple circular layout
    const allNodes = Array.from(nodeSet.values());
    const cx = 200;
    const cy = 150;
    const radius = Math.min(120, allNodes.length * 20);
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
    <div className="space-y-3">
      <h2 className="text-sm font-medium text-text-muted uppercase tracking-wider flex items-center gap-2">
        <GitBranch className="h-4 w-4" /> Symbol Graph
      </h2>

      <div className="flex items-center gap-2">
        <select
          value={corpusId}
          onChange={(e) => setCorpusId(e.target.value)}
          className="text-xs bg-surface-raised border border-border rounded px-2 py-1.5"
        >
          {status.corpora.map((c) => (
            <option key={c.id} value={c.id}>
              {c.id}
            </option>
          ))}
        </select>
      </div>

      <form
        onSubmit={(e) => {
          e.preventDefault();
          searchSymbols();
        }}
        className="flex gap-2"
      >
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search symbols..."
          className="flex-1 text-sm bg-surface-raised border border-border rounded px-3 py-1.5 placeholder:text-text-dim/50 focus:outline-none focus:ring-1 focus:ring-accent"
        />
        <button
          type="submit"
          disabled={loading}
          className="px-3 py-1.5 text-sm bg-accent text-white rounded hover:bg-accent/90 disabled:opacity-50 cursor-pointer"
        >
          <Search className="h-4 w-4" />
        </button>
      </form>

      <div className="flex gap-3 min-h-[300px]">
        {/* Symbol list */}
        <div className="w-64 shrink-0 space-y-1 overflow-y-auto max-h-[400px]">
          {symbols.map((s) => (
            <button
              key={s.id}
              onClick={() => selectSymbol(s)}
              className={`w-full text-left px-2 py-1.5 rounded text-xs transition-colors cursor-pointer ${
                selected?.id === s.id
                  ? "bg-accent/10 text-accent"
                  : "hover:bg-surface-overlay text-text"
              }`}
            >
              <div className="flex items-center gap-1.5">
                <Code2 className="h-3 w-3 shrink-0" />
                <span className="font-medium truncate">{s.name}</span>
              </div>
              <span className="text-text-dim ml-4.5">{s.kind}</span>
            </button>
          ))}
        </div>

        {/* Graph SVG */}
        <div className="flex-1 bg-surface-raised rounded border border-border overflow-hidden">
          {selected ? (
            <svg viewBox="0 0 400 300" className="w-full h-full">
              {/* Edges */}
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
                    stroke="currentColor"
                    strokeOpacity={0.2}
                    strokeWidth={1}
                  />
                );
              })}

              {/* Nodes */}
              {nodes.map((n) => {
                const isCenter = n.name === selected.name;
                return (
                  <g key={n.name}>
                    <circle
                      cx={n.x}
                      cy={n.y}
                      r={isCenter ? 8 : 5}
                      className={isCenter ? "fill-accent" : "fill-text-dim"}
                      opacity={isCenter ? 1 : 0.6}
                    />
                    <text
                      x={n.x}
                      y={n.y + (isCenter ? 18 : 14)}
                      textAnchor="middle"
                      className="fill-text text-[8px]"
                    >
                      {n.name.length > 20 ? n.name.slice(0, 20) + "…" : n.name}
                    </text>
                  </g>
                );
              })}
            </svg>
          ) : (
            <div className="flex items-center justify-center h-full text-text-dim text-xs">
              Select a symbol to view its reference graph
            </div>
          )}
        </div>
      </div>

      {/* Reference details */}
      {refs.length > 0 && (
        <div className="space-y-1">
          <h3 className="text-xs font-medium text-text-muted">
            References ({refs.length})
          </h3>
          <div className="max-h-40 overflow-y-auto space-y-1">
            {refs.map((r, i) => (
              <div
                key={i}
                className="flex items-center gap-2 text-xs text-text-dim bg-surface-raised rounded px-2 py-1"
              >
                <span className="font-mono">{r.from_name}</span>
                <span className="text-accent">→</span>
                <span className="font-mono">{r.to_name}</span>
                <span className="ml-auto px-1.5 py-0.5 rounded bg-surface-overlay text-text-dim">
                  {r.ref_kind}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
