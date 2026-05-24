import { Suspense } from "react";
import { CallbackHandler } from "./callback-handler";

export const metadata = {
  title: "Signing in… — ministr",
};

export default function AuthCallbackPage() {
  return (
    <Suspense
      fallback={
        <main className="ministr-v2" style={{ padding: "6rem 1.5rem", textAlign: "center" }}>
          <p style={{ fontFamily: "var(--font-mono), monospace", fontSize: "0.875rem", color: "var(--ink-2)" }}>
            Completing sign-in&hellip;
          </p>
        </main>
      }
    >
      <CallbackHandler />
    </Suspense>
  );
}
