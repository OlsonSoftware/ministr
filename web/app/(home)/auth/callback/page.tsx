import { Suspense } from "react";
import { CallbackHandler } from "./callback-handler";

export const metadata = {
  title: "Signing in... - ministr",
};

export default function AuthCallbackPage() {
  return (
    <div className="ministr-v2">
      <Suspense
        fallback={
          <section className="v2-section" style={{ paddingTop: '64px', textAlign: 'center' }}>
            <p
              style={{
                fontFamily: "var(--font-mono), monospace",
                fontSize: "14px",
                color: "var(--ink-2)",
              }}
            >
              Completing sign-in...
            </p>
          </section>
        }
      >
        <CallbackHandler />
      </Suspense>
    </div>
  );
}
