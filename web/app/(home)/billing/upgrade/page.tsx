import Link from 'next/link';

export default function UpgradePage() {
  return (
    <>
      <section className="v2-section">
        <p className="v2-label">Billing</p>
        <h1 className="v2-h2">Cloud plans coming soon</h1>
        <p className="v2-sub">
          ministr Cloud (hosted indexing, private repos, Atlas, team features) is
          not yet available. The local stack is free and fully featured today.
        </p>
      </section>

      <hr className="v2-rule" />

      <section className="v2-section">
        <p className="v2-prose">
          <Link href="/install" style={{ color: 'var(--amber)' }}>Install ministr</Link> to get
          started with the full local tool surface, or{' '}
          <Link href="/pricing" style={{ color: 'var(--amber)' }}>read about what is coming</Link>.
        </p>
      </section>
    </>
  );
}

export const metadata = {
  title: 'Upgrade',
  description: 'ministr cloud plans coming soon.',
};
