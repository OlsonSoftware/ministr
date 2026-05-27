import { Suspense } from "react";
import { AuthBridgePage } from "./auth-bridge-page";

export const metadata = {
  title: 'Bridge graph',
  description:
    "Authenticated cross-language bridge graph visualizer for your org corpora.",
};

export default function OrgBridgePage() {
  return (
    <Suspense
      fallback={
        <main className="ministr-v2" style={{ padding: "4rem 1.5rem" }}>
          <p
            style={{
              fontFamily: "var(--font-mono), monospace",
              fontSize: "0.875rem",
              color: "var(--ink-2)",
            }}
          >
            Loading…
          </p>
        </main>
      }
    >
      <AuthBridgePage />
    </Suspense>
  );
}
