import { LoginForm } from "./login-form";

export default function LoginPage() {
  return (
    <>
      <section className="v2-section" style={{ paddingBottom: 0 }}>
        <p className="v2-meta" style={{ marginBottom: 16 }}>Account</p>
        <h1 className="v2-h2">Sign in to ministr</h1>
        <p className="v2-sub">
          Paste an API key from the ministr desktop app (Settings &rarr; Cloud
          &rarr; API keys) to access authenticated pages.
        </p>
      </section>

      <hr className="v2-rule" />

      <section className="v2-section">
        <div style={{ maxWidth: '28rem' }}>
          <LoginForm />
        </div>
      </section>
    </>
  );
}

export const metadata = {
  title: 'Sign in',
  description: 'Sign in to ministr with GitHub or an API key.',
};
