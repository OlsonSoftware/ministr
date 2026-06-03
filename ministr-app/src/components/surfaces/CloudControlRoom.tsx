/**
 * CloudControlRoom — the Account area's dashboard-first CLOUD control room
 * (AAA-VISION OOUX, aaa-cloud).
 *
 * Cloud stops being a parallel destination that re-manages corpora + sessions.
 * The project-level "is this synced / shared" question is already a Tend
 * attribute; the genuinely-GLOBAL cloud bits live here in one thin place:
 *   · connection as a LIVING status (endpoint, health pulse, latency, plan)
 *   · usage as a real economics DASHBOARD (period total, per-kind, trend)
 *   · cloud corpora as ASSETS (health + consumers), not a manager
 *   · keys/webhooks as INFRA (counts + a way in), not a form
 *
 * Built fresh on the v4 tokens + ui/ atoms (Card, Badge, MetricTile, Sparkline,
 * StatusDot, Button, EmptyState) — it is NOT a re-skin of the retired
 * CloudPanel. The pure `CloudControlRoom` renders from props so Storybook can
 * drive anon / authed / empty without Tauri; `CloudControlRoomConnector` wires
 * the live `cloudClient`.
 */
import { useEffect, useMemo, useState } from "react";
import {
  Activity,
  Boxes,
  CloudOff,
  Gauge,
  KeyRound,
  Plug,
  Radio,
  RefreshCw,
  Webhook,
} from "lucide-react";

import type {
  CloudCorpusInfo,
  CloudStatus,
  CloudUsage,
} from "../../lib/cloudClient";
import { cloudClient } from "../../lib/cloudClient";
import { cn } from "../../lib/utils";

import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Card } from "../ui/card";
import { EmptyState } from "../ui/empty-state";
import { MetricTile } from "../ui/metric-tile";
import { Sparkline } from "../ui/sparkline";
import { StatusDot } from "../ui/status-dot";

export interface CloudControlRoomProps {
  /** Connection snapshot. `null` while the first status load is in flight. */
  connection: CloudStatus | null;
  /** Metered usage for the period. `null` when anon or not yet loaded. */
  usage: CloudUsage | null;
  /** Cloud-side corpora (assets). */
  corpora: CloudCorpusInfo[];
  /** Active service-account API keys. */
  apiKeyCount: number;
  /** Webhook subscriptions across the caller's orgs. */
  webhookCount: number;
  /** First status load still pending. */
  loading?: boolean;
  onConnect: () => void;
  onDisconnect: () => void;
  onManageBilling: () => void;
  onRefresh?: () => void;
}

const PLAN_TONE: Record<CloudUsage["plan"], "default" | "success" | "muted"> = {
  pro: "muted",
  team: "default",
  enterprise: "success",
};

/** Sum rollup + partial rows by metric kind into one ordered economics table. */
function aggregateUsage(usage: CloudUsage | null) {
  if (!usage) return { total: 0, today: 0, perKind: [] as Array<{ kind: string; total: number }>, series: [] as number[] };

  const byDay = new Map<string, number>();
  const byKind = new Map<string, number>();
  let total = 0;
  for (const r of usage.rollups) {
    byDay.set(r.day, (byDay.get(r.day) ?? 0) + r.total);
    byKind.set(r.kind, (byKind.get(r.kind) ?? 0) + r.total);
    total += r.total;
  }
  let today = 0;
  for (const t of usage.today_partial) {
    byKind.set(t.kind, (byKind.get(t.kind) ?? 0) + t.total);
    today += t.total;
    total += t.total;
  }

  const series = [...byDay.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([, v]) => v);
  if (today > 0) series.push(today);

  const perKind = [...byKind.entries()]
    .map(([kind, t]) => ({ kind, total: t }))
    .sort((a, b) => b.total - a.total);

  return { total, today, perKind, series };
}

function corpusAssetTone(status?: string | null): {
  variant: "success" | "warning" | "danger" | "muted";
  label: string;
} {
  const s = (status ?? "").toLowerCase();
  if (s === "ready" || s === "idle" || s === "complete")
    return { variant: "success", label: "Ready" };
  if (s === "indexing" || s === "running" || s === "pending")
    return { variant: "warning", label: "Indexing" };
  if (s === "error" || s === "failed")
    return { variant: "danger", label: "Error" };
  return { variant: "muted", label: status ?? "Unknown" };
}

/** A labelled section — the repeated shell each control-room concern lives in. */
function Section({
  icon: Icon,
  title,
  meta,
  children,
}: {
  icon: typeof Gauge;
  title: string;
  meta?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="space-y-2.5">
      <div className="flex items-center gap-2">
        <Icon className="h-3.5 w-3.5 text-accent" strokeWidth={2.25} />
        <h3 className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
          {title}
        </h3>
        {meta && <div className="ml-auto">{meta}</div>}
      </div>
      {children}
    </section>
  );
}

export function CloudControlRoom({
  connection,
  usage,
  corpora,
  apiKeyCount,
  webhookCount,
  loading = false,
  onConnect,
  onDisconnect,
  onManageBilling,
  onRefresh,
}: CloudControlRoomProps) {
  const econ = useMemo(() => aggregateUsage(usage), [usage]);

  // ── Loading — the first status load is in flight. ──────────────────────────
  if (loading && !connection) {
    return (
      <div className="h-full grid place-items-center p-8">
        <div className="flex items-center gap-2 font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
          <Radio className="h-3.5 w-3.5 animate-pulse" strokeWidth={2} />
          Contacting cloud…
        </div>
      </div>
    );
  }

  // ── Anonymous — not connected. One clear call to value, not a form. ────────
  const authed = !!connection?.authenticated;
  if (!authed) {
    return (
      <div className="h-full grid place-items-center p-8">
        <div className="max-w-sm text-center space-y-5">
          <div className="mx-auto grid h-14 w-14 place-items-center rounded-2xl border border-border bg-surface-overlay text-text-dim">
            <CloudOff className="h-6 w-6" strokeWidth={1.75} />
          </div>
          <div className="space-y-1.5">
            <h2 className="font-sans text-base font-semibold text-text">
              Connect ministr Cloud
            </h2>
            <p className="font-sans text-sm text-text-dim leading-relaxed">
              Sync indexes across machines, share them with your org, and run
              agents against a hosted endpoint. Everything stays local until you
              connect.
            </p>
          </div>
          <div className="flex flex-wrap items-center justify-center gap-1.5">
            {["Sync", "Share", "Usage", "API keys"].map((c) => (
              <Badge key={c} variant="muted">
                {c}
              </Badge>
            ))}
          </div>
          <Button onClick={onConnect} className="mx-auto">
            <Plug className="h-4 w-4" strokeWidth={2} />
            Sign in to connect
          </Button>
          {connection?.endpoint && (
            <p className="font-mono text-[10px] text-text-dim truncate">
              {connection.endpoint}
            </p>
          )}
        </div>
      </div>
    );
  }

  // ── Connected — the dashboard. ─────────────────────────────────────────────
  const healthy = connection?.last_health_ok === true;
  const healthTone = healthy ? "success" : connection?.last_health_ok === false ? "danger" : "muted";
  const latency = connection?.last_health_latency_ms;
  const plan = usage?.plan;

  return (
    <div className="h-full overflow-y-auto px-5 py-5 space-y-6">
      {/* ── Connection — a living status, not a settings row. ─────────────── */}
      <Card className="p-4">
        <div className="flex items-center gap-3">
          <StatusDot tone={healthTone} pulse={healthy ? "live" : "off"} size="md" />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="font-sans text-sm font-semibold text-text">
                {healthy ? "Connected" : connection?.last_health_ok === false ? "Connection degraded" : "Connection unknown"}
              </span>
              {plan && (
                <Badge variant={PLAN_TONE[plan]}>{plan}</Badge>
              )}
            </div>
            <p className="font-mono text-[10px] text-text-dim truncate mt-0.5">
              {connection?.endpoint}
              {typeof latency === "number" && (
                <span className="text-text-muted"> · {latency}ms</span>
              )}
            </p>
          </div>
          <div className="flex items-center gap-1.5 shrink-0">
            {onRefresh && (
              <Button variant="ghost" size="icon-sm" onClick={onRefresh} aria-label="Refresh cloud status">
                <RefreshCw className="h-3.5 w-3.5" strokeWidth={2} />
              </Button>
            )}
            <Button variant="subtle" size="sm" onClick={onDisconnect}>
              Disconnect
            </Button>
          </div>
        </div>
      </Card>

      {/* ── Usage — a real economics dashboard. ──────────────────────────── */}
      <Section
        icon={Gauge}
        title="Usage"
        meta={
          <Button variant="ghost" size="sm" onClick={onManageBilling}>
            Manage billing
          </Button>
        }
      >
        {usage && econ.total > 0 ? (
          <Card className="p-4">
            <div className="flex items-end justify-between gap-4">
              <div>
                <p className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
                  Operations · period
                </p>
                <p className="font-mono text-2xl font-semibold tabular-nums text-text mt-0.5">
                  {econ.total.toLocaleString()}
                </p>
                <p className="font-mono text-[10px] text-text-dim mt-0.5">
                  {econ.today.toLocaleString()} today
                </p>
              </div>
              {econ.series.length > 1 && (
                <Sparkline
                  data={econ.series}
                  smooth
                  tone="accent"
                  width={180}
                  height={48}
                  ariaLabel="Daily cloud operations trend over the billing period"
                />
              )}
            </div>
            {econ.perKind.length > 0 && (
              <div className="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-3">
                {econ.perKind.slice(0, 6).map((k) => (
                  <MetricTile
                    key={k.kind}
                    variant="compact"
                    label={k.kind}
                    value={k.total.toLocaleString()}
                  />
                ))}
              </div>
            )}
          </Card>
        ) : (
          <Card className="p-4">
            <p className="font-sans text-sm text-text-dim">
              No metered usage yet this period. Run a survey or ask against a
              cloud corpus and it shows up here.
            </p>
          </Card>
        )}
      </Section>

      {/* ── Corpora — ASSETS with health + consumers. ────────────────────── */}
      <Section
        icon={Boxes}
        title="Cloud corpora"
        meta={
          corpora.length > 0 && (
            <span className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim tabular-nums">
              {corpora.length}
            </span>
          )
        }
      >
        {corpora.length === 0 ? (
          <EmptyState
            icon={Boxes}
            title="No cloud corpora"
            hint="Indexes you sync to the cloud appear here as assets — with their health and how many agents are using them."
          />
        ) : (
          <ul className="space-y-2">
            {corpora.map((c) => {
              const asset = corpusAssetTone(c.indexing_status);
              const consumers = c.active_sessions ?? 0;
              return (
                <li key={c.corpus_id}>
                  <Card hover="lift" className="p-3.5">
                    <div className="flex items-center gap-3">
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <span className="font-sans text-sm font-medium text-text truncate">
                            {c.display_name ?? c.corpus_id}
                          </span>
                          <Badge variant={asset.variant} dot>
                            {asset.label}
                          </Badge>
                        </div>
                        <p className="font-mono text-[10px] text-text-dim truncate mt-0.5">
                          {c.paths[0] ?? c.corpus_id}
                        </p>
                      </div>
                      <div className="flex items-center gap-4 shrink-0">
                        <MetricTile
                          variant="compact"
                          label="files"
                          value={(c.total_files ?? 0).toLocaleString()}
                        />
                        <MetricTile
                          variant="compact"
                          label="chunks"
                          value={(c.total_chunks ?? 0).toLocaleString()}
                        />
                        <MetricTile
                          variant="compact"
                          icon={consumers > 0 ? Activity : undefined}
                          tone={consumers > 0 ? "accent" : undefined}
                          label="consumers"
                          value={consumers.toLocaleString()}
                        />
                      </div>
                    </div>
                  </Card>
                </li>
              );
            })}
          </ul>
        )}
      </Section>

      {/* ── Infra — keys + webhooks as machine surfaces, kept thin. ───────── */}
      <Section icon={Plug} title="Automation">
        <div className="grid grid-cols-2 gap-2">
          <Card className="p-3.5">
            <div className="flex items-center gap-3">
              <span className="grid h-8 w-8 place-items-center rounded-md border border-border bg-surface-overlay text-text-muted shrink-0">
                <KeyRound className="h-3.5 w-3.5" strokeWidth={2} />
              </span>
              <div className="min-w-0">
                <p className="font-mono text-base font-semibold tabular-nums text-text leading-none">
                  {apiKeyCount.toLocaleString()}
                </p>
                <p className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim mt-1">
                  API keys
                </p>
              </div>
            </div>
          </Card>
          <Card className="p-3.5">
            <div className="flex items-center gap-3">
              <span className="grid h-8 w-8 place-items-center rounded-md border border-border bg-surface-overlay text-text-muted shrink-0">
                <Webhook className="h-3.5 w-3.5" strokeWidth={2} />
              </span>
              <div className="min-w-0">
                <p className="font-mono text-base font-semibold tabular-nums text-text leading-none">
                  {webhookCount.toLocaleString()}
                </p>
                <p className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim mt-1">
                  Webhooks
                </p>
              </div>
            </div>
          </Card>
        </div>
      </Section>
    </div>
  );
}

// ───────────────────────────────────────────────────────────────────────────
// Connector — wires the live cloudClient. Reads are defensive (a 403 on a
// non-owner usage/webhook call must not blank the whole dashboard).

export function CloudControlRoomConnector({
  onManageBilling,
}: {
  onManageBilling: () => void;
}) {
  const [connection, setConnection] = useState<CloudStatus | null>(null);
  const [usage, setUsage] = useState<CloudUsage | null>(null);
  const [corpora, setCorpora] = useState<CloudCorpusInfo[]>([]);
  const [apiKeyCount, setApiKeyCount] = useState(0);
  const [webhookCount, setWebhookCount] = useState(0);
  const [loading, setLoading] = useState(true);
  const [nonce, setNonce] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);

    (async () => {
      let status: CloudStatus | null = null;
      try {
        status = await cloudClient.status();
      } catch {
        status = null;
      }
      if (cancelled) return;
      setConnection(status);

      if (!status?.authenticated) {
        setUsage(null);
        setCorpora([]);
        setApiKeyCount(0);
        setWebhookCount(0);
        setLoading(false);
        return;
      }

      const [u, cs, keys, orgs] = await Promise.all([
        cloudClient.billingUsage().catch(() => null),
        cloudClient.listCorpora().catch(() => [] as CloudCorpusInfo[]),
        cloudClient.listApiKeys().catch(() => []),
        cloudClient.listOrgs().catch(() => []),
      ]);
      if (cancelled) return;
      setUsage(u);
      setCorpora(cs);
      setApiKeyCount(keys.length);

      // Webhooks live per-org; sum across orgs the caller can read. Owner-only
      // on the server, so a 403 simply contributes zero.
      const counts = await Promise.all(
        orgs.map((o) => cloudClient.listWebhookSubs(o.id).then((w) => w.length).catch(() => 0)),
      );
      if (cancelled) return;
      setWebhookCount(counts.reduce((a, b) => a + b, 0));
      setLoading(false);
    })();

    return () => {
      cancelled = true;
    };
  }, [nonce]);

  return (
    <CloudControlRoom
      connection={connection}
      usage={usage}
      corpora={corpora}
      apiKeyCount={apiKeyCount}
      webhookCount={webhookCount}
      loading={loading}
      onConnect={() => {
        void cloudClient.authenticate().then(() => setNonce((n) => n + 1));
      }}
      onDisconnect={() => {
        void cloudClient.disconnect().then(() => setNonce((n) => n + 1));
      }}
      onManageBilling={onManageBilling}
      onRefresh={() => setNonce((n) => n + 1)}
    />
  );
}
