"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { useSearchParams } from "next/navigation";
import { AuthGate } from "@/components/auth-gate";
import { useAuth } from "@/lib/auth";

interface Org {
  id: string;
  name: string;
  plan_id: string;
  role: string;
}

interface RollupRow {
  user_id: string;
  email: string;
  day: string;
  kind: string;
  total: number;
}

interface PartialRow {
  user_id: string;
  email: string;
  kind: string;
  total: number;
}

interface OrgUsage {
  org_id: string;
  range_days: number;
  rollups: RollupRow[];
  today_partial: PartialRow[];
}

interface MemberTotals {
  email: string;
  rollup: Map<string, number>;
  partial: Map<string, number>;
}

const KINDS = ["query.served", "index.minutes", "atlas.queries"] as const;

function kindLabel(k: string): string {
  switch (k) {
    case "query.served":
      return "Queries";
    case "index.minutes":
      return "Index min";
    case "atlas.queries":
      return "Atlas";
    default:
      return k;
  }
}

function fmt(n: number): string {
  return n.toLocaleString();
}

export function OrgUsagePage() {
  const params = useSearchParams();
  const orgParam = params?.get("org") ?? null;

  return (
    <main className="ministr-v2">
      <div
        style={{
          maxWidth: "60rem",
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
            Team &middot; Usage dashboard
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
            Org usage
          </h1>
        </header>

        <AuthGate>
          <OrgUsageDashboard initialOrgId={orgParam} />
        </AuthGate>
      </div>
    </main>
  );
}

function OrgUsageDashboard({
  initialOrgId,
}: {
  initialOrgId: string | null;
}) {
  const { token, endpoint } = useAuth();
  const [orgs, setOrgs] = useState<Org[]>([]);
  const [selectedOrg, setSelectedOrg] = useState<string | null>(initialOrgId);
  const [usage, setUsage] = useState<OrgUsage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const headers = useMemo((): Record<string, string> => {
    const h: Record<string, string> = { Accept: "application/json" };
    if (token) h.Authorization = `Bearer ${token}`;
    return h;
  }, [token]);
  const base = endpoint.replace(/\/$/, "");

  useEffect(() => {
    if (!token) return;
    fetch(`${base}/api/v1/orgs`, { headers })
      .then(async (r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        const data = (await r.json()) as Org[];
        setOrgs(data);
        if (!selectedOrg && data.length > 0) setSelectedOrg(data[0].id);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [token, base, headers, selectedOrg]);

  const fetchUsage = useCallback(async () => {
    if (!selectedOrg || !token) return;
    setLoading(true);
    setError(null);
    try {
      const r = await fetch(
        `${base}/api/v1/orgs/${encodeURIComponent(selectedOrg)}/usage?days=30`,
        { headers },
      );
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      setUsage(await r.json());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [selectedOrg, token, base, headers]);

  useEffect(() => {
    void fetchUsage();
  }, [fetchUsage]);

  const members = useMemo(() => {
    if (!usage) return [];
    const map = new Map<string, MemberTotals>();
    for (const r of usage.rollups) {
      let m = map.get(r.user_id);
      if (!m) {
        m = { email: r.email, rollup: new Map(), partial: new Map() };
        map.set(r.user_id, m);
      }
      m.rollup.set(r.kind, (m.rollup.get(r.kind) ?? 0) + r.total);
    }
    for (const p of usage.today_partial) {
      let m = map.get(p.user_id);
      if (!m) {
        m = { email: p.email, rollup: new Map(), partial: new Map() };
        map.set(p.user_id, m);
      }
      m.partial.set(p.kind, (m.partial.get(p.kind) ?? 0) + p.total);
    }
    return Array.from(map.entries()).map(([id, t]) => ({ id, ...t }));
  }, [usage]);

  const orgTotals = useMemo(() => {
    const rollup = new Map<string, number>();
    const partial = new Map<string, number>();
    for (const m of members) {
      for (const [k, v] of m.rollup) rollup.set(k, (rollup.get(k) ?? 0) + v);
      for (const [k, v] of m.partial)
        partial.set(k, (partial.get(k) ?? 0) + v);
    }
    return { rollup, partial };
  }, [members]);

  const mono =
    "fontFamily: var(--font-mono), monospace; fontSize: 0.75rem;" as const;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1rem" }}>
      {/* Org picker */}
      {orgs.length > 1 && (
        <select
          value={selectedOrg ?? ""}
          onChange={(e) => setSelectedOrg(e.target.value || null)}
          style={{
            fontFamily: "var(--font-mono), monospace",
            fontSize: "0.75rem",
            padding: "0.5rem 0.75rem",
            borderRadius: "0.5rem",
            border: "1px solid var(--rule)",
            background: "var(--bg-2)",
            color: "var(--ink)",
            maxWidth: "24rem",
          }}
        >
          {orgs.map((o) => (
            <option key={o.id} value={o.id}>
              {o.name} ({o.role})
            </option>
          ))}
        </select>
      )}

      {error && (
        <p
          style={{
            fontFamily: "var(--font-mono), monospace",
            fontSize: "0.75rem",
            color: "#ef4444",
          }}
        >
          {error}
        </p>
      )}

      {loading && (
        <p
          style={{
            fontFamily: "var(--font-mono), monospace",
            fontSize: "0.75rem",
            color: "var(--amber)",
          }}
        >
          Loading usage&hellip;
        </p>
      )}

      {usage && !loading && (
        <>
          {/* Totals header */}
          <div
            style={{
              display: "flex",
              gap: "2rem",
              padding: "1rem",
              borderRadius: "0.5rem",
              border: "1px solid var(--rule)",
              background: "var(--bg-2)",
              flexWrap: "wrap",
            }}
          >
            <Stat label="Members" value={fmt(members.length)} />
            {KINDS.map((k) => {
              const r = orgTotals.rollup.get(k) ?? 0;
              const p = orgTotals.partial.get(k) ?? 0;
              return (
                <Stat
                  key={k}
                  label={kindLabel(k)}
                  value={p > 0 ? `${fmt(r)} (+${fmt(p)} today)` : fmt(r)}
                />
              );
            })}
            <Stat label="Period" value={`${usage.range_days}d`} />
          </div>

          {/* Members table */}
          <div
            style={{
              borderRadius: "0.5rem",
              border: "1px solid var(--rule)",
              overflow: "auto",
            }}
          >
            <table
              style={{
                width: "100%",
                borderCollapse: "collapse",
                fontFamily: "var(--font-mono), monospace",
                fontSize: "0.75rem",
              }}
            >
              <thead>
                <tr
                  style={{
                    borderBottom: "1px solid var(--rule)",
                    background: "var(--bg-2)",
                  }}
                >
                  <th style={th}>Member</th>
                  {KINDS.map((k) => (
                    <th key={k} style={{ ...th, textAlign: "right" }}>
                      {kindLabel(k)}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {members.map((m) => (
                  <tr
                    key={m.id}
                    style={{ borderBottom: "1px solid var(--rule-2, var(--rule))" }}
                  >
                    <td style={td}>{m.email}</td>
                    {KINDS.map((k) => {
                      const r = m.rollup.get(k) ?? 0;
                      const p = m.partial.get(k) ?? 0;
                      return (
                        <td key={k} style={{ ...td, textAlign: "right" }}>
                          {p > 0 ? (
                            <>
                              {fmt(r)}{" "}
                              <span style={{ color: "var(--amber)" }}>
                                (+{fmt(p)})
                              </span>
                            </>
                          ) : (
                            fmt(r)
                          )}
                        </td>
                      );
                    })}
                  </tr>
                ))}
                {members.length === 0 && (
                  <tr>
                    <td
                      colSpan={KINDS.length + 1}
                      style={{ ...td, textAlign: "center", color: "var(--muted)" }}
                    >
                      No usage data for this period.
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        </>
      )}
    </div>
  );
}

const th: React.CSSProperties = {
  padding: "0.5rem 0.75rem",
  textAlign: "left",
  fontWeight: 600,
  textTransform: "uppercase",
  letterSpacing: "0.08em",
  color: "var(--ink-2)",
  fontSize: "0.6875rem",
};

const td: React.CSSProperties = {
  padding: "0.5rem 0.75rem",
  color: "var(--ink)",
};

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div
        style={{
          fontFamily: "var(--font-mono), monospace",
          fontSize: "0.6875rem",
          textTransform: "uppercase",
          letterSpacing: "0.08em",
          color: "var(--muted)",
        }}
      >
        {label}
      </div>
      <div
        style={{
          fontFamily: "var(--font-mono), monospace",
          fontSize: "0.875rem",
          fontWeight: 600,
          color: "var(--ink)",
          marginTop: "0.125rem",
        }}
      >
        {value}
      </div>
    </div>
  );
}
