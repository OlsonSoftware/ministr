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
    <div className="ministr-v2">
      <section className="v2-section" style={{ paddingTop: '64px' }}>
        <p className="v2-meta" style={{ marginBottom: '16px' }}>Status</p>
        <h1 className="v2-h2" style={{ maxWidth: 'none' }}>Cloud SLA snapshot</h1>
        <p className="v2-sub">
          Live data from <code>{baseUrl}/sla</code>, refreshed every 30 seconds.
          Contractual SLA targets: ≥{SLA_TARGET_UPTIME_PCT}% uptime, p95 ≤{' '}
          {SLA_TARGET_P95_MS}ms query latency.
        </p>
      </section>

      <hr className="v2-rule" />

      <section className="v2-section">
        {sla ? <SlaCards sla={sla} /> : <DegradedState baseUrl={baseUrl} />}
      </section>

      <footer className="v2-footer">
        <div style={{ color: 'var(--muted)', fontSize: '12px', fontFamily: 'var(--font-mono), monospace' }}>
          Server-side fetch, cached 30s. Pipeline: skeleton → latency → persist-write → persist-read.
        </div>
        <div className="v2-footer-links">
          <a href="/">Home</a>
          <a href="/pricing">Pricing</a>
        </div>
      </footer>
    </div>
  );
}

function SlaCards({ sla }: { sla: SlaResponse }) {
  const latency = sla.latency;
  const currentP95Ms = latency?.p95_ms;
  const meetingP95 = currentP95Ms !== undefined && currentP95Ms <= SLA_TARGET_P95_MS;

  return (
    <div className="v2-features">
      <div className="v2-feature">
        <h3>Uptime</h3>
        <p style={{ fontSize: '28px', fontWeight: 500, color: 'var(--ink)', marginBottom: '8px' }}>
          {formatUptime(sla.uptime_secs)}
        </p>
        <p>Since boot · {sla.started_at_iso}</p>
        <p>Version {sla.version}</p>
      </div>

      <div className="v2-feature">
        <h3>Current p95</h3>
        {latency ? (
          <>
            <p style={{
              fontSize: '28px',
              fontWeight: 500,
              color: meetingP95 ? '#34d399' : 'var(--amber)',
              marginBottom: '8px',
            }}>
              {latency.p95_ms}ms
            </p>
            <p>p50: {latency.p50_ms}ms · p99: {latency.p99_ms}ms</p>
            <p>{latency.count} samples</p>
          </>
        ) : (
          <p>No samples yet — backend just booted.</p>
        )}
      </div>

      <div className="v2-feature">
        <h3>30-day worst p95</h3>
        {latency?.window_30d_max_p95_ms != null ? (
          <p style={{
            fontSize: '28px',
            fontWeight: 500,
            color: latency.window_30d_max_p95_ms <= SLA_TARGET_P95_MS ? '#34d399' : 'var(--amber)',
            marginBottom: '8px',
          }}>
            {latency.window_30d_max_p95_ms}ms
          </p>
        ) : (
          <p>No persisted snapshots in the 30-day window.</p>
        )}
      </div>

      <div className="v2-feature">
        <h3>SLA contract</h3>
        <p>Uptime target: ≥{SLA_TARGET_UPTIME_PCT}%</p>
        <p>p95 target: ≤{SLA_TARGET_P95_MS}ms</p>
        <p style={{ marginTop: '8px', fontSize: '13px' }}>
          Sustained breaches trigger SLA credits per the Enterprise agreement.
        </p>
      </div>
    </div>
  );
}

function DegradedState({ baseUrl }: { baseUrl: string }) {
  return (
    <div style={{
      border: '1px solid var(--rule)',
      padding: '32px',
      color: 'var(--ink-2)',
    }}>
      <h2 style={{ color: 'var(--amber)', fontSize: '16px', fontWeight: 500, marginBottom: '14px' }}>
        Backend unreachable
      </h2>
      <p>
        The /sla endpoint at <code>{baseUrl}/sla</code> did not respond.
        Check back in 30 seconds.
      </p>
    </div>
  );
}

export const metadata = {
  title: 'Status — ministr',
  description:
    'ministr cloud SLA snapshot: uptime, latency percentiles, and historical worst p95.',
};
