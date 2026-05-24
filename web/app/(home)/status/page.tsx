// F5.5-b-page-skeleton — public /status page.
//
// Polls the cloud's `/sla` endpoint (F5.5-b-sla-skeleton +
// F5.5-b-latency + F5.5-b-persist-read) every 30s via Next.js fetch
// revalidation and renders the uptime envelope + current p50/p95/p99 +
// the rolling 30-day worst p95.
//
// Server component; zero client JS. Stale-when-revalidate keeps the
// page snappy even if a request races the next backend tick.

import {
  DEFAULT_CLOUD_BASE_URL,
  fetchSlaStatus,
  formatUptime,
  type SlaResponse,
} from '@/lib/status';

export const revalidate = 30;

const SLA_TARGET_P95_MS = 200;
const SLA_TARGET_UPTIME_PCT = 99.5;

export default async function StatusPage() {
  const baseUrl =
    process.env.NEXT_PUBLIC_MINISTR_CLOUD_BASE_URL ?? DEFAULT_CLOUD_BASE_URL;
  const sla = await fetchSlaStatus(baseUrl);

  return (
    <main className="mx-auto flex max-w-4xl flex-col gap-10 p-8">
      <header className="flex flex-col gap-3">
        <p className="font-mono text-xs uppercase tracking-[0.18em] text-fd-muted-foreground">
          Status
        </p>
        <h1 className="text-3xl font-semibold sm:text-4xl">
          Cloud SLA snapshot
        </h1>
        <p className="max-w-3xl text-fd-muted-foreground">
          Live data from <code className="font-mono">{baseUrl}/sla</code>,
          refreshed every 30 seconds. Contractual SLA targets: ≥
          {SLA_TARGET_UPTIME_PCT}% uptime, p95 ≤ {SLA_TARGET_P95_MS}ms query
          latency.
        </p>
      </header>

      {sla ? <SlaCards sla={sla} /> : <DegradedState baseUrl={baseUrl} />}

      <footer className="border-t border-fd-border pt-6 text-xs text-fd-muted-foreground">
        <p>
          The data above is fetched server-side and cached for 30 seconds. For
          the full historical record, see the operator dashboard (deferred —
          this page is the customer-facing summary). Snapshot pipeline:
          F5.5-b-skeleton → F5.5-b-latency → F5.5-b-persist-write →
          F5.5-b-persist-read.
        </p>
      </footer>
    </main>
  );
}

function SlaCards({ sla }: { sla: SlaResponse }) {
  const latency = sla.latency;
  const currentP95Ms = latency?.p95_ms;
  const meetingP95 =
    currentP95Ms !== undefined && currentP95Ms <= SLA_TARGET_P95_MS;
  return (
    <div className="grid grid-cols-1 gap-6 sm:grid-cols-2">
      <Card title="Uptime">
        <Metric value={formatUptime(sla.uptime_secs)} caption="since boot" />
        <Field label="Booted at" value={sla.started_at_iso} mono />
        <Field label="Version" value={sla.version} mono />
      </Card>

      <Card title="Current request latency">
        {latency ? (
          <>
            <Metric
              value={`${latency.p95_ms}ms`}
              caption={`p95 of last ${latency.count} requests`}
              tone={meetingP95 ? 'ok' : 'warn'}
            />
            <Field label="p50" value={`${latency.p50_ms}ms`} mono />
            <Field label="p99" value={`${latency.p99_ms}ms`} mono />
          </>
        ) : (
          <p className="text-sm text-fd-muted-foreground">
            No samples yet — backend just booted or has seen no traffic.
          </p>
        )}
      </Card>

      <Card title="Historical worst p95 (30-day window)">
        {latency?.window_30d_max_p95_ms != null ? (
          <Metric
            value={`${latency.window_30d_max_p95_ms}ms`}
            caption="max p95 across persisted snapshots"
            tone={
              latency.window_30d_max_p95_ms <= SLA_TARGET_P95_MS ? 'ok' : 'warn'
            }
          />
        ) : (
          <p className="text-sm text-fd-muted-foreground">
            No persisted snapshots in the 30-day window. This serve may be
            self-hosted (no DB-backed store wired) or recently restarted.
          </p>
        )}
      </Card>

      <Card title="SLA contract">
        <Field label="Uptime target" value={`≥ ${SLA_TARGET_UPTIME_PCT}%`} />
        <Field label="p95 target" value={`≤ ${SLA_TARGET_P95_MS}ms`} />
        <p className="mt-2 text-xs text-fd-muted-foreground">
          The 30-day rolling worst p95 above is the contractual measurement
          point — sustained breaches trigger SLA credits per the Enterprise
          agreement.
        </p>
      </Card>
    </div>
  );
}

function DegradedState({ baseUrl }: { baseUrl: string }) {
  return (
    <section className="rounded-lg border border-fd-border bg-fd-muted/40 p-6">
      <h2 className="text-lg font-medium">Backend unreachable</h2>
      <p className="mt-2 text-sm text-fd-muted-foreground">
        The /sla endpoint at <code className="font-mono">{baseUrl}/sla</code>{' '}
        did not respond with valid JSON. This could be a transient backend
        blip, a maintenance window, or a misconfigured deployment. Check back
        in 30 seconds.
      </p>
    </section>
  );
}

function Card({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="flex flex-col gap-3 rounded-lg border border-fd-border p-6">
      <h2 className="text-sm font-medium uppercase tracking-wide text-fd-muted-foreground">
        {title}
      </h2>
      {children}
    </section>
  );
}

function Metric({
  value,
  caption,
  tone,
}: {
  value: string;
  caption: string;
  tone?: 'ok' | 'warn';
}) {
  const toneClass =
    tone === 'warn'
      ? 'text-amber-600 dark:text-amber-400'
      : tone === 'ok'
        ? 'text-emerald-600 dark:text-emerald-400'
        : '';
  return (
    <div className="flex flex-col gap-1">
      <span className={`text-3xl font-semibold ${toneClass}`}>{value}</span>
      <span className="text-xs text-fd-muted-foreground">{caption}</span>
    </div>
  );
}

function Field({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="flex items-center justify-between text-sm">
      <span className="text-fd-muted-foreground">{label}</span>
      <span className={mono ? 'font-mono text-xs' : ''}>{value}</span>
    </div>
  );
}
