"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { useSearchParams } from "next/navigation";
import { AuthGate } from "@/components/auth-gate";
import { useAuth } from "@/lib/auth";
import { BridgeGraphInteractive } from "@/components/bridge/bridge-graph-interactive";
import { BridgeGraphFilters } from "@/components/bridge/bridge-graph-filters";
import { BridgeGraphSidePanel } from "@/components/bridge/bridge-graph-side-panel";
import {
  applyBridgeFilters,
  distinctLanguages,
  distinctKinds,
  noFilters,
  type BridgeFilters,
} from "@/components/bridge/bridge-filters";
import {
  BRIDGE_GRAPH_SAMPLE,
  type LiveBridgeNode,
  type LiveBridgeEdge,
} from "@/components/landing/bridge-graph";

export function AuthBridgePage() {
  const params = useSearchParams();
  const corpus = params?.get("corpus") ?? null;

  return (
    <main className="ministr-v2">
      <div
        style={{
          maxWidth: "80rem",
          margin: "0 auto",
          padding: "2rem 1.5rem",
          display: "flex",
          flexDirection: "column",
          gap: "1.5rem",
        }}
      >
        <header>
          <p
            style={{
              fontFamily: "var(--font-mono), monospace",
              fontSize: "0.6875rem",
              textTransform: "uppercase",
              letterSpacing: "0.18em",
              color: "var(--muted)",
            }}
          >
            Team &middot; Bridge visualizer
          </p>
          <h1
            style={{
              fontFamily: "var(--font-geist), sans-serif",
              fontSize: "1.5rem",
              fontWeight: 600,
              color: "var(--ink)",
              marginTop: "0.375rem",
            }}
          >
            Bridge graph
          </h1>
          {!corpus && (
            <p
              style={{
                fontFamily: "var(--font-mono), monospace",
                fontSize: "0.75rem",
                color: "var(--ink-2)",
                marginTop: "0.5rem",
                lineHeight: 1.6,
              }}
            >
              Add <code>?corpus=your-corpus-id</code> to the URL to load a live
              bridge graph from your cloud account.
            </p>
          )}
        </header>

        <AuthGate>
          <section
            style={{
              borderRadius: "0.5rem",
              border: "1px solid var(--rule)",
              background: "var(--bg-2)",
              padding: "1rem",
            }}
          >
            {corpus ? (
              <AuthBridgeGraph corpusId={corpus} />
            ) : (
              <BridgeGraphInteractive data={BRIDGE_GRAPH_SAMPLE} />
            )}
          </section>
        </AuthGate>
      </div>
    </main>
  );
}

function AuthBridgeGraph({ corpusId }: { corpusId: string }) {
  const { token, endpoint } = useAuth();
  const [data, setData] = useState<{
    nodes: ReadonlyArray<LiveBridgeNode>;
    edges: ReadonlyArray<LiveBridgeEdge>;
  }>(BRIDGE_GRAPH_SAMPLE);
  const [status, setStatus] = useState<
    "idle" | "loading" | "success" | "error"
  >("idle");
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [filters, setFilters] = useState<BridgeFilters>(noFilters);
  const [selectedEdge, setSelectedEdge] = useState<LiveBridgeEdge | null>(null);

  useEffect(() => {
    if (!token) return;
    const url = `${endpoint.replace(/\/$/, "")}/api/v1/corpora/${encodeURIComponent(corpusId)}/bridge/graph`;
    setStatus("loading");
    const controller = new AbortController();
    fetch(url, {
      headers: {
        Accept: "application/json",
        Authorization: `Bearer ${token}`,
      },
      signal: controller.signal,
    })
      .then(async (resp) => {
        if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
        const json = await resp.json();
        if (!Array.isArray(json.nodes) || !Array.isArray(json.edges))
          throw new Error("malformed response");
        setData({ nodes: json.nodes, edges: json.edges });
        setStatus("success");
      })
      .catch((err) => {
        if (err?.name === "AbortError") return;
        setErrorMsg(err instanceof Error ? err.message : String(err));
        setStatus("error");
      });
    return () => controller.abort();
  }, [token, endpoint, corpusId]);

  const availableLanguages = useMemo(() => distinctLanguages(data), [data]);
  const availableKinds = useMemo(() => distinctKinds(data), [data]);
  const filteredData = useMemo(
    () => applyBridgeFilters(data, filters),
    [data, filters],
  );
  const filteredNodesById = useMemo(
    () => new Map(filteredData.nodes.map((n) => [n.id, n])),
    [filteredData],
  );

  const apiContext = useMemo(
    () =>
      token ? { api: endpoint, id: corpusId, token } : null,
    [token, endpoint, corpusId],
  );

  const clearEdge = useCallback(() => setSelectedEdge(null), []);

  useEffect(() => {
    if (!selectedEdge) return;
    const still = filteredData.edges.some(
      (e) =>
        e.from === selectedEdge.from &&
        e.to === selectedEdge.to &&
        e.kind === selectedEdge.kind,
    );
    if (!still) setSelectedEdge(null);
  }, [filteredData, selectedEdge]);

  return (
    <>
      {status === "loading" && (
        <p
          style={{
            fontFamily: "var(--font-mono), monospace",
            fontSize: "0.75rem",
            color: "var(--amber)",
            marginBottom: "0.5rem",
          }}
        >
          Fetching bridge graph&hellip;
        </p>
      )}
      {status === "error" && (
        <p
          style={{
            fontFamily: "var(--font-mono), monospace",
            fontSize: "0.75rem",
            color: "#ef4444",
            marginBottom: "0.5rem",
          }}
        >
          {errorMsg}
        </p>
      )}
      {status === "success" && (
        <p
          style={{
            fontFamily: "var(--font-mono), monospace",
            fontSize: "0.75rem",
            color: "#22c55e",
            marginBottom: "0.5rem",
          }}
        >
          Live data from {endpoint}
        </p>
      )}
      <BridgeGraphFilters
        filters={filters}
        onChange={setFilters}
        availableLanguages={availableLanguages}
        availableKinds={availableKinds}
      />
      <div className="grid gap-4 lg:grid-cols-[1fr_22rem]">
        <BridgeGraphInteractive
          data={filteredData}
          onEdgeClick={setSelectedEdge}
        />
        {selectedEdge ? (
          <BridgeGraphSidePanel
            edge={selectedEdge}
            nodesById={filteredNodesById}
            apiContext={apiContext}
            onClose={clearEdge}
          />
        ) : null}
      </div>
    </>
  );
}
