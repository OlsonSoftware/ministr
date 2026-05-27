import Link from 'next/link';

export default function ManageBillingPage() {
  return (
    <>
      <section className="v2-section">
        <p className="v2-label">Billing</p>
        <h1 className="v2-h2">Manage billing</h1>
        <p className="v2-sub">
          Cloud billing is not yet available. When it launches, this page will
          link to your Stripe Customer Portal.
        </p>
      </section>

      <hr className="v2-rule" />

      <section className="v2-section">
        <p className="v2-prose">
          <Link href="/install" style={{ color: 'var(--amber)' }}>Install ministr</Link> to get
          started with the full local tool surface.
        </p>
      </section>
    </>
  );
}

export const metadata = {
  title: 'Manage billing',
  description: 'ministr cloud billing management.',
};
