import { Suspense } from "react";
import { OrgUsagePage } from "./org-usage-page";

export const metadata = {
  title: "Org usage — ministr",
  description: "Per-member usage dashboard for your ministr org.",
};

export default function Page() {
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
      <OrgUsagePage />
    </Suspense>
  );
}
