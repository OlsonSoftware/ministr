import { LoginForm } from "./login-form";

export default function LoginPage() {
  return (
    <div className="ministr-v2">
      <div
        style={{
          maxWidth: "28rem",
          margin: "0 auto",
          padding: "6rem 1.5rem 4rem",
        }}
      >
        <h1
          style={{
            fontFamily: "var(--font-geist), sans-serif",
            fontSize: "1.5rem",
            fontWeight: 600,
            color: "var(--ink)",
            marginBottom: "0.5rem",
          }}
        >
          Sign in to ministr
        </h1>
        <p
          style={{
            fontFamily: "var(--font-geist), sans-serif",
            fontSize: "0.875rem",
            color: "var(--ink-2)",
            marginBottom: "2rem",
            lineHeight: 1.6,
          }}
        >
          Paste an API key from the ministr desktop app (Settings &rarr; Cloud
          &rarr; API keys) to access authenticated pages.
        </p>

        <LoginForm />
      </div>
    </div>
  );
}
