import { LoginForm } from "./login-form";

export default function LoginPage() {
  return (
    <div className="ministr-v2">
      <section className="v2-section" style={{ paddingTop: '64px' }}>
        <p className="v2-meta" style={{ marginBottom: '16px' }}>Account</p>
        <h1 className="v2-h2" style={{ maxWidth: 'none' }}>Sign in to ministr</h1>
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

      <footer className="v2-footer">
        <div className="v2-footer-links">
          <a href="/">Home</a>
          <a href="/docs">Docs</a>
        </div>
      </footer>
    </div>
  );
}

export const metadata = {
  title: 'Sign in - ministr',
  description: 'Sign in to ministr with GitHub or an API key.',
};
